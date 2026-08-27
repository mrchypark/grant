use axum::{
    Router,
    routing::{any, delete, get, post, put},
};
use infra::service::AppService;
use std::sync::Arc;

mod authz;
mod backups;
mod dto;
mod error;
mod handlers;
mod http_capabilities;
mod mappers;
mod operation_catalog;
mod patch_data_operations;
mod rauthy_proxy;
mod request;

use authz::{
    application_authz_batch_check, application_authz_check, application_authz_filter,
    authz_batch_check, authz_check, authz_filter, exposure_read_authorization,
    operation_authorization_v2, plant_read_authorization,
};
use backups::{list_backups_local, list_backups_s3, trigger_backup};
use handlers::{
    add_group_member, bootstrap_external_owner, bootstrap_rauthy_owner, create_agreement,
    create_application, create_group, create_role, create_role_binding, deactivate_organization,
    delete_group, delete_role_binding, enroll_organization_member, get_agreement, get_application,
    get_application_consent, get_current_user, get_group, healthz, import_authority_snapshot_v2,
    import_patch_data_authority, list_agreements, list_application_actions,
    list_application_grants, list_applications, list_group_members, list_groups, list_providers,
    list_role_bindings, list_roles, materialize_current_user, reactivate_organization, readyz,
    remove_group_member, replace_application_actions, replace_application_consent,
    replace_provider_actions, replace_role_actions, requires_voter_node,
    revoke_application_consent, set_application_trust, set_user_active, update_agreement,
    update_group, upsert_provider, verify_application_provisioning,
};
use http_capabilities::{
    activate_http_capability_generation, decide_http_capability,
    get_active_http_capability_catalog, set_provider_pep_client, stage_http_capability_generation,
};
use rauthy_proxy::{auth_root, proxy_rauthy_passthrough, proxy_rauthy_root};

pub fn build_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route("/api/v1/me/materialize", post(materialize_current_user))
        .route("/api/v1/admin/providers", post(upsert_provider))
        .route(
            "/api/v1/admin/providers/{providerId}/actions",
            put(replace_provider_actions),
        )
        .route(
            "/api/v1/admin/providers/{providerId}/http-capability-generations",
            post(stage_http_capability_generation),
        )
        .route(
            "/api/v1/admin/providers/{providerId}/http-capability-active",
            post(activate_http_capability_generation),
        )
        .route(
            "/api/v1/admin/providers/{providerId}/pep-clients/{pepClientId}",
            put(set_provider_pep_client),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps",
            post(create_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            put(replace_application_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent",
            put(replace_application_consent).delete(revoke_application_consent),
        )
        .route("/api/v1/orgs/{organizationId}/groups", post(create_group))
        .route(
            "/api/v1/orgs/{organizationId}/groups/{groupId}",
            put(update_group).delete(delete_group),
        )
        .route(
            "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}",
            put(add_group_member).delete(remove_group_member),
        )
        .route(
            "/api/v1/orgs/{organizationId}/members/{userId}",
            put(enroll_organization_member),
        )
        .route(
            "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust",
            put(set_application_trust),
        )
        .route(
            "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/provisioning/verify",
            post(verify_application_provisioning),
        )
        .route("/api/v1/admin/users/{userId}/active", put(set_user_active))
        .route(
            "/api/v1/orgs/{organizationId}/agreements",
            post(create_agreement),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements/{agreementId}",
            put(update_agreement),
        )
        .route("/api/v1/orgs/{organizationId}/roles", post(create_role))
        .route(
            "/api/v1/orgs/{organizationId}/roles/{roleId}/actions",
            put(replace_role_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings",
            post(create_role_binding),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings/{bindingId}",
            delete(delete_role_binding),
        )
        .route(
            "/api/v1/orgs/{organizationId}/deactivate",
            post(deactivate_organization),
        )
        .route(
            "/api/v1/orgs/{organizationId}/reactivate",
            post(reactivate_organization),
        )
        .route(
            "/api/v1/admin/backups",
            get(list_backups_local).post(trigger_backup),
        )
        .route("/api/v1/admin/backups/s3", get(list_backups_s3))
        .route(
            "/api/v1/admin/bootstrap/rauthy-owner",
            post(bootstrap_rauthy_owner),
        )
        .route(
            "/api/v1/admin/bootstrap/external-owner",
            post(bootstrap_external_owner),
        )
        .route(
            "/api/v1/admin/imports/patch-data-authority",
            post(import_patch_data_authority),
        )
        .route(
            "/api/v2/admin/imports/authority-snapshots",
            post(import_authority_snapshot_v2),
        )
        .with_state(service)
}

pub fn build_learner_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route("/api/v1/admin/providers", any(requires_voter_node))
        .route(
            "/api/v1/admin/providers/{providerId}/actions",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/admin/providers/{providerId}/http-capability-generations",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/admin/providers/{providerId}/http-capability-active",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/admin/providers/{providerId}/pep-clients/{pepClientId}",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent",
            put(requires_voter_node).delete(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/groups",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/groups/{groupId}",
            put(requires_voter_node).delete(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/groups/{groupId}/members/{userId}",
            put(requires_voter_node).delete(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/members/{userId}",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/trust",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/admin/orgs/{organizationId}/apps/{applicationId}/provisioning/verify",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/admin/users/{userId}/active",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements/{agreementId}",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/roles",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/roles/{roleId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/bindings/{bindingId}",
            delete(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/deactivate",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/reactivate",
            post(requires_voter_node),
        )
        .route("/api/v1/admin/backups", any(requires_voter_node))
        .route("/api/v1/admin/backups/s3", any(requires_voter_node))
        .route("/api/v1/me/materialize", post(requires_voter_node))
        .route(
            "/api/v1/admin/bootstrap/rauthy-owner",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/admin/bootstrap/external-owner",
            any(requires_voter_node),
        )
        .route(
            "/api/v1/admin/imports/patch-data-authority",
            any(requires_voter_node),
        )
        .with_state(service)
}

fn build_shared_router() -> Router<Arc<AppService>> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/auth", get(auth_root))
        .route("/auth/v1", any(proxy_rauthy_root))
        .route("/auth/v1/{*rauthyPath}", any(proxy_rauthy_passthrough))
        .route("/api/v1/me", get(get_current_user))
        .route("/api/v1/providers", get(list_providers))
        .route(
            "/api/v2/http-capabilities/catalog/{providerId}",
            get(get_active_http_capability_catalog),
        )
        .route(
            "/api/v2/http-capabilities/decide",
            post(decide_http_capability),
        )
        .route("/api/v1/orgs/{organizationId}/apps", get(list_applications))
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}",
            get(get_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            get(list_application_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants",
            get(list_application_grants),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/consent",
            get(get_application_consent),
        )
        .route("/api/v1/orgs/{organizationId}/groups", get(list_groups))
        .route(
            "/api/v1/orgs/{organizationId}/groups/{groupId}",
            get(get_group),
        )
        .route(
            "/api/v1/orgs/{organizationId}/groups/{groupId}/members",
            get(list_group_members),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements",
            get(list_agreements),
        )
        .route(
            "/api/v1/orgs/{organizationId}/agreements/{agreementId}",
            get(get_agreement),
        )
        .route("/api/v1/orgs/{organizationId}/roles", get(list_roles))
        .route(
            "/api/v1/orgs/{organizationId}/bindings",
            get(list_role_bindings),
        )
        .route("/api/v1/authz/check", post(authz_check))
        .route("/api/v1/authz/batch-check", post(authz_batch_check))
        .route("/api/v1/authz/filter", post(authz_filter))
        .route(
            "/api/v1/application-authz/check",
            post(application_authz_check),
        )
        .route(
            "/api/v1/application-authz/batch-check",
            post(application_authz_batch_check),
        )
        .route(
            "/api/v1/application-authz/filter",
            post(application_authz_filter),
        )
        .route(
            "/api/v1/exposure-authz/read",
            post(exposure_read_authorization),
        )
        .route(
            "/api/v1/exposure-authz/plant-read",
            post(plant_read_authorization),
        )
        .route(
            "/api/v2/exposure-authz/decide",
            post(operation_authorization_v2),
        )
}

#[cfg(test)]
mod tests {
    use super::error::ApiError;
    use anyhow::anyhow;
    use axum::http::StatusCode;
    use infra::service::{ServiceError, ServiceErrorKind};

    #[test]
    fn auth_unavailable_errors_map_to_service_unavailable() {
        let err = ApiError::from_auth(anyhow!("unavailable: quorum read path is not ready"));
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn domain_unavailable_errors_map_to_service_unavailable() {
        let err =
            ApiError::from_domain(anyhow!("unavailable: quorum validation path is not ready"));
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn structured_service_errors_override_string_matching() {
        let err = ApiError::from_domain(anyhow::Error::new(ServiceError::new(
            ServiceErrorKind::BadRequest,
            "challengeSignatureBase64 must not be empty",
        )));
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.message, "challengeSignatureBase64 must not be empty");
    }

    #[test]
    fn application_security_service_errors_map_to_http_statuses() {
        for (kind, expected_status) in [
            (ServiceErrorKind::Forbidden, StatusCode::FORBIDDEN),
            (ServiceErrorKind::NotFound, StatusCode::NOT_FOUND),
            (ServiceErrorKind::Conflict, StatusCode::CONFLICT),
        ] {
            let err = ApiError::from_domain(anyhow::Error::new(ServiceError::new(
                kind,
                "application security mutation failed",
            )));
            assert_eq!(err.status, expected_status);
        }
    }

    #[test]
    fn generic_invalid_domain_errors_stay_internal() {
        let err = ApiError::from_domain(anyhow!("invalid internal state"));
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
