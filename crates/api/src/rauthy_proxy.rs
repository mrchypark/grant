use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{StatusCode, header},
    response::{Redirect, Response},
};
use infra::service::AppService;
use std::sync::Arc;

use crate::error::{ApiError, ApiResult};

pub(crate) async fn auth_root() -> Redirect {
    Redirect::temporary("/auth/v1")
}

pub(crate) async fn proxy_rauthy_root(
    State(service): State<Arc<AppService>>,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    proxy_rauthy_request(service.as_ref(), request, "/auth/v1").await
}

pub(crate) async fn proxy_rauthy_passthrough(
    State(service): State<Arc<AppService>>,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    let upstream_path = request.uri().path().to_owned();
    proxy_rauthy_request(service.as_ref(), request, &upstream_path).await
}

async fn proxy_rauthy_request(
    service: &AppService,
    request: axum::extract::Request,
    upstream_path: &str,
) -> ApiResult<Response> {
    let (parts, body) = request.into_parts();
    let mut url = format!(
        "{}{}",
        service.rauthy_base_url().trim_end_matches('/'),
        upstream_path
    );
    if let Some(query) = parts.uri.query() {
        url.push('?');
        url.push_str(query);
    }

    let body_bytes = to_bytes(body, 10 * 1024 * 1024)
        .await
        .map_err(ApiError::internal)?;
    let mut upstream = service
        .rauthy_proxy_client()
        .request(parts.method.clone(), url);
    for (name, value) in &parts.headers {
        if name == header::HOST || name == header::CONTENT_LENGTH {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    let upstream_response = upstream
        .body(body_bytes.clone())
        .send()
        .await
        .map_err(ApiError::internal)?;

    let status = upstream_response.status();
    let headers = upstream_response.headers().clone();
    let bytes = upstream_response
        .bytes()
        .await
        .map_err(ApiError::internal)?;

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::from_u16(status.as_u16()).map_err(ApiError::internal)?;
    let response_headers = response.headers_mut();
    for (name, value) in &headers {
        if name == header::CONTENT_LENGTH {
            continue;
        }
        response_headers.append(name, value.clone());
    }
    Ok(response)
}
