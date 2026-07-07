use crate::{
    config::ReadConsistency,
    runtime::{HiqliteRuntime, Readiness},
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use domain::{
    Action, Agreement, Application, ApplicationAction, ApplicationGrant, ApplicationGrantAction,
    ApplicationGrantStatus, ApplicationRedirectUri, HealthStatus, Organization, Role, RoleBinding,
    RoleBindingState, RoleBindingSubjectType, ServiceProvider, User,
};
use hiqlite::Row;
use std::borrow::Cow;
use std::collections::HashSet;
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
mod identity;
mod organizations;
mod roles;
mod schema;

use codec::*;
use rows::*;

const MAX_AUTHZ_CHECKS_PER_QUERY: usize = 100;

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
    pub consistency: Option<ReadConsistency>,
}

#[derive(Debug, Clone)]
pub struct CachedExternalIdentity {
    pub provider: String,
    pub external_user_id: String,
    pub email: Option<String>,
    pub display_name: String,
    pub active: bool,
    pub expires_at: DateTime<Utc>,
}
