use crate::{
    config::ReadConsistency,
    runtime::{HiqliteRuntime, Readiness},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use domain::{
    Action, Agreement, Application, ApplicationAction, ApplicationGrant, ApplicationGrantAction,
    ApplicationGrantStatus, ApplicationProvisioningStatus, ApplicationRedirectUri, Group,
    GroupMember, HealthStatus, Organization, Role, RoleBinding, RoleBindingState,
    RoleBindingSubjectType, ServiceProvider, User,
};
use hiqlite::Row;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    error::Error as StdError,
    fmt,
};
use uuid::Uuid;

mod codec;
mod rows;

#[macro_use]
mod core;

mod agreements;
mod application_grants;
mod applications;
mod authz;
mod backups;
mod catalog;
mod field_views;
mod groups;
mod identity;
mod organizations;
mod roles;
mod schema;

use codec::*;
use rows::*;

// Each check binds twelve SQLite parameters. Stay below conservative 999-variable builds.
const MAX_AUTHZ_CHECKS_PER_QUERY: usize = 80;
// Bound policy expansion per check. Overflow is a configuration/capacity error, not a partial result.
const MAX_CANDIDATE_ROLES_PER_CHECK: usize = 256;

#[derive(Clone)]
pub struct Database {
    runtime: HiqliteRuntime,
    bootstrap_user_id: String,
    bootstrap_user_name: String,
}

#[derive(Debug, Clone)]
pub struct BackupEntry {
    pub name: String,
    pub last_modified: i64,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationConsentWriteError {
    PreconditionsChanged,
}

impl fmt::Display for ApplicationConsentWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreconditionsChanged => f.write_str(
                "application consent preconditions changed while consent was being written",
            ),
        }
    }
}

impl StdError for ApplicationConsentWriteError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRoleLimitExceeded {
    pub check_index: usize,
    pub candidate_roles: usize,
    pub limit: usize,
}

impl fmt::Display for CandidateRoleLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "authorization check {} expanded to {} candidate roles; limit is {}",
            self.check_index, self.candidate_roles, self.limit
        )
    }
}

impl StdError for CandidateRoleLimitExceeded {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationTrustMutationError {
    SecurityMutationInProgress { application_id: String },
}

impl fmt::Display for ApplicationTrustMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SecurityMutationInProgress { application_id } => write!(
                f,
                "application security mutation already in progress: {application_id}"
            ),
        }
    }
}

impl StdError for ApplicationTrustMutationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationSecurityMutationError {
    ApplicationNotFound { application_id: String },
    TrustedApplication { application_id: String },
    LeaseExpiredOrInvalid { application_id: String },
}

impl fmt::Display for ApplicationSecurityMutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApplicationNotFound { application_id } => {
                write!(f, "application not found: {application_id}")
            }
            Self::TrustedApplication { .. } => f.write_str(
                "forbidden: trusted applications may only be changed by the master administrator",
            ),
            Self::LeaseExpiredOrInvalid { application_id } => write!(
                f,
                "application security lease expired or invalid: {application_id}"
            ),
        }
    }
}

impl StdError for ApplicationSecurityMutationError {}

#[derive(Debug, Clone, Copy)]
pub struct BindIdentityInput<'a> {
    pub user_id: &'a str,
    pub provider: &'a str,
    pub external_user_id: &'a str,
    pub email: Option<&'a str>,
    pub display_name: &'a str,
    pub active: bool,
    pub allow_rebind: bool,
}

#[derive(Debug, Clone)]
pub struct UserActionCheck<'a> {
    pub user_id: &'a str,
    pub organization_id: &'a str,
    pub provider_id: &'a str,
    pub resource_type: &'a str,
    pub resource_id: &'a str,
    pub action_id: &'a str,
    pub application_id: Option<&'a str>,
    pub consistency: Option<ReadConsistency>,
}

#[derive(Debug, Clone, Copy)]
pub struct UserActionFieldViewCheck<'a> {
    pub schema_key: &'a str,
    pub fields: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserActionDecision {
    pub allowed: bool,
    pub rbac_allowed: bool,
    pub trusted_application: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserActionEvaluation {
    pub decision: UserActionDecision,
    pub field_views: Option<BTreeMap<String, domain::FieldViewMode>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleActionFieldViewPolicy {
    pub role_id: String,
    pub action_id: String,
    pub resource_type: String,
    pub schema_key: String,
    pub field_views: BTreeMap<String, domain::FieldViewMode>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReplaceRoleActionFieldViewPolicy {
    pub organization_id: String,
    pub actor_user_id: String,
    pub role_id: String,
    pub action_id: String,
    pub resource_type: String,
    pub schema_key: String,
    pub field_views: BTreeMap<String, domain::FieldViewMode>,
    pub idempotency_key: String,
    pub request_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceRoleActionFieldViewPolicyOutcome {
    Stored(RoleActionFieldViewPolicy),
    IdempotencyConflict,
}

#[derive(Debug, Clone)]
pub struct CachedExternalIdentity {
    pub provider: String,
    pub external_user_id: String,
    pub client_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
    pub expires_at: DateTime<Utc>,
}
