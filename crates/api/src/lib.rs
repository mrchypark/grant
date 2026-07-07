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
mod mappers;
mod rauthy_proxy;
mod request;

use authz::{authz_batch_check, authz_check, authz_filter};
use backups::{list_backups_local, list_backups_s3, trigger_backup};
use handlers::{
    bootstrap_rauthy_owner, create_agreement, create_application, create_application_grant,
    create_role, create_role_binding, deactivate_organization, delete_role_binding, get_agreement,
    get_application, get_current_user, healthz, list_agreements, list_application_actions,
    list_application_grants, list_applications, list_providers, list_role_bindings, list_roles,
    reactivate_organization, replace_application_actions, replace_application_grant_actions,
    replace_provider_actions, replace_role_actions, requires_voter_node, revoke_application_grant,
    update_agreement, update_application, upsert_provider,
};
use rauthy_proxy::{auth_root, proxy_rauthy_passthrough, proxy_rauthy_root};

pub fn build_router(service: Arc<AppService>) -> Router {
    build_shared_router()
        .route("/api/v1/admin/providers", post(upsert_provider))
        .route(
            "/api/v1/admin/providers/{providerId}/actions",
            put(replace_provider_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps",
            post(create_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}",
            put(update_application),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            put(replace_application_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants",
            post(create_application_grant),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/actions",
            put(replace_application_grant_actions),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/revoke",
            post(revoke_application_grant),
        )
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
            "/api/v1/orgs/{organizationId}/apps",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/apps/{applicationId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants",
            post(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/actions",
            put(requires_voter_node),
        )
        .route(
            "/api/v1/orgs/{organizationId}/app-grants/{grantId}/revoke",
            post(requires_voter_node),
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
        .route(
            "/api/v1/admin/bootstrap/rauthy-owner",
            any(requires_voter_node),
        )
        .with_state(service)
}

fn build_shared_router() -> Router<Arc<AppService>> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/auth", get(auth_root))
        .route("/auth/v1", any(proxy_rauthy_root))
        .route("/auth/v1/{*rauthyPath}", any(proxy_rauthy_passthrough))
        .route("/api/v1/me", get(get_current_user))
        .route("/api/v1/providers", get(list_providers))
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
    fn generic_invalid_domain_errors_stay_internal() {
        let err = ApiError::from_domain(anyhow!("invalid internal state"));
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
