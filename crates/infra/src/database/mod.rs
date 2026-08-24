use crate::{
    config::ReadConsistency,
    runtime::{HiqliteRuntime, Readiness},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use domain::{
    Action, Agreement, Application, ApplicationAction, ApplicationGrant, ApplicationGrantAction,
    ApplicationGrantStatus, ApplicationProvisioningStatus, ApplicationRedirectUri, Group,
    GroupMember, HealthStatus, MembershipRole, MembershipStatus, Organization,
    OrganizationMembership, Role, RoleBinding, RoleBindingState, RoleBindingSubjectType,
    ServiceProvider, User,
};
use hiqlite::Row;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashSet, error::Error as StdError, fmt};
use uuid::Uuid;

mod codec;
mod rows;

#[macro_use]
mod core;

mod agreements;
mod application_grants;
mod applications;
mod authority_imports;
pub use authority_imports::patch_data_manifest_digest;
mod authz;
mod backups;
mod catalog;
mod groups;
mod identity;
mod organizations;
mod roles;
mod schema;

use codec::*;
use rows::*;

// Each check binds ten SQLite parameters. Stay below conservative 999-variable builds.
const MAX_AUTHZ_CHECKS_PER_QUERY: usize = 90;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizationMembershipWriteError {
    TargetUserNotFound,
    TargetUserInactive,
    OrganizationInactive,
    ExistingNonMemberRole,
    PreconditionsChanged,
}

impl fmt::Display for OrganizationMembershipWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetUserNotFound => {
                f.write_str("organization membership target user was not found")
            }
            Self::TargetUserInactive => {
                f.write_str("organization membership target user is inactive")
            }
            Self::OrganizationInactive => f.write_str("organization is inactive"),
            Self::ExistingNonMemberRole => {
                f.write_str("organization membership already has a non-member role")
            }
            Self::PreconditionsChanged => {
                f.write_str("organization membership enrollment preconditions changed")
            }
        }
    }
}

impl StdError for OrganizationMembershipWriteError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalOwnerBootstrapError {
    ExposureApplicationNotReady,
    ExistingMemberRole,
    ExistingAdminRole,
    ExistingOwner { user_id: String },
    PreconditionsChanged,
}

impl fmt::Display for ExternalOwnerBootstrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExposureApplicationNotReady => {
                f.write_str("configured Patch Data exposure application is not ready")
            }
            Self::ExistingMemberRole => {
                f.write_str("external owner bootstrap cannot promote an existing member")
            }
            Self::ExistingAdminRole => {
                f.write_str("external owner bootstrap cannot promote an existing admin")
            }
            Self::ExistingOwner { user_id } => {
                write!(
                    f,
                    "external owner bootstrap organization already has owner {user_id}"
                )
            }
            Self::PreconditionsChanged => {
                f.write_str("external owner bootstrap preconditions changed")
            }
        }
    }
}

impl StdError for ExternalOwnerBootstrapError {}

/// The only data needed to materialize an externally verified identity.  Keep
/// this deliberately small: callers must never select a local user by email
/// or by the provider's bare subject.
#[derive(Debug, Clone)]
pub struct ExternalIdentityMaterialization<'a> {
    pub provider: &'a str,
    pub external_user_id: &'a str,
    pub email: Option<&'a str>,
    pub display_name: &'a str,
}

#[derive(Debug, Clone)]
pub struct ExternalOwnerBootstrapCommand<'a> {
    pub identity: ExternalIdentityMaterialization<'a>,
    pub organization_id: &'a str,
    pub application_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalOwnerBootstrapResult {
    pub user_id: String,
    pub organization_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataHierarchyEdge {
    pub parent_organization_id: String,
    pub child_organization_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataImportedUser {
    pub user_id: String,
    pub external_user_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataImportedOrganization {
    pub organization_id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataImportedMembership {
    pub membership_id: String,
    pub organization_id: String,
    pub user_id: String,
    pub role: MembershipRole,
    pub status: MembershipStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataImportedPlantBinding {
    pub binding_id: String,
    pub subject_type: RoleBindingSubjectType,
    pub subject_id: String,
    pub subject_organization_id: String,
    pub plant_id: String,
    pub state: RoleBindingState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataImportedApplicationAssignment {
    pub application_id: String,
    pub organization_id: String,
}

/// Complete, deterministic Patch Data authorization snapshot. Imported rows
/// are materialized into Grant's canonical identity, membership and binding
/// tables; Patch Data is never consulted by the steady-state evaluator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PatchDataAuthorityImport {
    pub manifest_version: String,
    pub generation: String,
    pub ledger_digest: String,
    pub manifest_digest: String,
    pub expected_previous_generation: Option<String>,
    pub users: Vec<PatchDataImportedUser>,
    pub organizations: Vec<PatchDataImportedOrganization>,
    pub memberships: Vec<PatchDataImportedMembership>,
    pub hierarchy_edges: Vec<PatchDataHierarchyEdge>,
    pub plant_bindings: Vec<PatchDataImportedPlantBinding>,
    pub application_assignments: Vec<PatchDataImportedApplicationAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchDataAuthorityImportResult {
    pub applied: bool,
    pub manifest_version: String,
    pub generation: String,
    pub ledger_digest: String,
    pub manifest_digest: String,
    pub applied_at: String,
}

/// A narrow seam for services and deterministic tests.  It intentionally does
/// not expose arbitrary identity updates or lookup-by-email operations.
pub trait ExternalIdentityStore {
    fn external_identity_store_marker(&self) {}
}

impl ExternalIdentityStore for Database {}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserActionDecision {
    pub allowed: bool,
    pub rbac_allowed: bool,
    pub trusted_application: bool,
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
