use anyhow::anyhow;
use axum::http::{HeaderMap, StatusCode, header};
use infra::{config::ReadConsistency, service::AppService};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    dto::ReadConsistencyQuery,
    error::{ApiError, ApiResult},
};

#[derive(Debug, Clone)]
pub(crate) struct RequestContext {
    pub(crate) user_id: String,
    pub(crate) organization_id: String,
}

pub(crate) async fn request_context(
    service: &AppService,
    headers: &HeaderMap,
    read_consistency: Option<ReadConsistency>,
) -> ApiResult<RequestContext> {
    let organization_id = header_value(headers, "x-organization-id")?;
    let bearer_token = bearer_token(headers)?;
    let principal = service
        .authenticate_request(
            bearer_token.as_deref(),
            Some(&organization_id),
            read_consistency,
        )
        .await
        .map_err(ApiError::from_auth)?;
    Ok(RequestContext {
        user_id: principal.user_id,
        organization_id: principal.organization_id,
    })
}

pub(crate) async fn request_context_for_inactive_organization(
    service: &AppService,
    headers: &HeaderMap,
    target_organization_id: &str,
    read_consistency: Option<ReadConsistency>,
) -> ApiResult<RequestContext> {
    let bearer_token = bearer_token(headers)?;
    let principal = service
        .authenticate_request_for_inactive_organization(
            bearer_token.as_deref(),
            target_organization_id,
            read_consistency,
        )
        .await
        .map_err(ApiError::from_domain)?;
    Ok(RequestContext {
        user_id: principal.user_id,
        organization_id: principal.organization_id,
    })
}

pub(crate) fn read_consistency_override(
    query: &ReadConsistencyQuery,
) -> ApiResult<Option<ReadConsistency>> {
    let Some(value) = query.consistency.as_deref() else {
        return Ok(None);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "strong" => Ok(Some(ReadConsistency::Strong)),
        other => Err(ApiError::bad_request(anyhow!(
            "invalid consistency: {other}"
        ))),
    }
}

pub(crate) fn ensure_active_organization_path_matches(
    path_organization_id: &str,
    active_organization_id: &str,
) -> ApiResult<()> {
    if path_organization_id == active_organization_id {
        return Ok(());
    }
    Err(ApiError {
        status: StatusCode::FORBIDDEN,
        message: format!(
            "forbidden: organization path {} does not match active organization {}",
            path_organization_id, active_organization_id
        ),
    })
}

pub(crate) fn ensure_request_context_organization(
    path_organization_id: &str,
    ctx: &RequestContext,
) -> ApiResult<()> {
    ensure_active_organization_path_matches(path_organization_id, &ctx.organization_id)
}

pub(crate) fn require_master_key(service: &AppService, headers: &HeaderMap) -> ApiResult<()> {
    let provided = header_value(headers, "x-grant-master-key")?;
    if !secrets_match(&provided, service.master_key()) {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "unauthorized: invalid master key".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn require_app_auth_key(service: &AppService, headers: &HeaderMap) -> ApiResult<()> {
    let provided = header_value(headers, "x-grant-app-auth-key")?;
    if !secrets_match(&provided, service.app_auth_key()) {
        return Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "unauthorized: invalid application authorization key".to_owned(),
        });
    }
    Ok(())
}

fn secrets_match(provided: &str, expected: &str) -> bool {
    let provided_digest = Sha256::digest(provided.as_bytes());
    let expected_digest = Sha256::digest(expected.as_bytes());
    bool::from(provided_digest.ct_eq(&expected_digest))
}

fn header_value(headers: &HeaderMap, key: &'static str) -> ApiResult<String> {
    optional_header_value(headers, key)?
        .ok_or_else(|| ApiError::bad_request(anyhow!("missing {key} header")))
}

pub(crate) fn optional_header_value(
    headers: &HeaderMap,
    key: &'static str,
) -> ApiResult<Option<String>> {
    let value = headers.get(key);
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request(anyhow!("{key} header is not valid UTF-8")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "{key} header must not be empty"
        )));
    }
    Ok(Some(trimmed.to_owned()))
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> ApiResult<Option<String>> {
    let value = match headers.get(header::AUTHORIZATION) {
        Some(value) => value,
        None => return Ok(None),
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request(anyhow!("authorization header is not valid UTF-8")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "authorization header must not be empty"
        )));
    }
    let token = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))
        .ok_or_else(|| ApiError::bad_request(anyhow!("authorization header must use Bearer")))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(ApiError::bad_request(anyhow!(
            "authorization bearer token must not be empty"
        )));
    }
    Ok(Some(token.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{ensure_active_organization_path_matches, secrets_match};
    use axum::http::StatusCode;

    #[test]
    fn organization_path_match_helper_returns_forbidden_on_mismatch() {
        let err = ensure_active_organization_path_matches("org-b", "org-a")
            .expect_err("mismatched organization path should be rejected");
        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert_eq!(
            err.message,
            "forbidden: organization path org-b does not match active organization org-a"
        );
    }

    #[test]
    fn secret_comparison_accepts_equal_values() {
        assert!(secrets_match("master-secret", "master-secret"));
    }

    #[test]
    fn secret_comparison_rejects_first_character_mismatch() {
        assert!(!secrets_match("Master-secret", "master-secret"));
    }

    #[test]
    fn secret_comparison_rejects_middle_character_mismatch() {
        assert!(!secrets_match("master-Secret", "master-secret"));
    }

    #[test]
    fn secret_comparison_rejects_last_character_mismatch() {
        assert!(!secrets_match("master-secreT", "master-secret"));
    }

    #[test]
    fn secret_comparison_rejects_length_mismatch() {
        assert!(!secrets_match("master-secret-extra", "master-secret"));
    }
}
