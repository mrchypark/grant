use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use infra::service::{ServiceError, ServiceErrorKind};

pub(crate) type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn requires_voter() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "endpoint requires voter node".to_owned(),
        }
    }

    pub(crate) fn bad_request(err: anyhow::Error) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: err.to_string(),
        }
    }

    pub(crate) fn internal<E>(err: E) -> Self
    where
        E: ToString,
    {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }

    pub(crate) fn from_domain(err: anyhow::Error) -> Self {
        if let Some(status) = structured_status(&err) {
            return Self {
                status,
                message: err.to_string(),
            };
        }
        let message = err.to_string();
        let contains_chain =
            |needle: &str| err.chain().any(|cause| cause.to_string().contains(needle));
        let status = if message.contains("forbidden:") {
            StatusCode::FORBIDDEN
        } else if contains_chain("already exists")
            || contains_chain("UNIQUE constraint failed")
            || contains_chain("unique constraint failed")
        {
            StatusCode::CONFLICT
        } else if message.contains("unavailable:") {
            StatusCode::SERVICE_UNAVAILABLE
        } else if contains_chain("not found") {
            StatusCode::NOT_FOUND
        } else if contains_chain("must not be empty")
            || contains_chain("is not grantable to applications")
            || contains_chain("duplicate action code in provider manifest")
            || contains_chain("duplicate action id in provider manifest")
            || contains_chain("expired")
            || contains_chain("consumed")
            || contains_chain("payload mismatch")
            || contains_chain("mismatch")
            || contains_chain("must be included")
            || contains_chain("already used")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, message }
    }

    pub(crate) fn from_auth(err: anyhow::Error) -> Self {
        if let Some(status) = structured_status(&err) {
            return Self {
                status,
                message: err.to_string(),
            };
        }
        let message = err.to_string();
        let status = if message.contains("unauthorized:") {
            StatusCode::UNAUTHORIZED
        } else if message.contains("forbidden:") {
            StatusCode::FORBIDDEN
        } else if message.contains("unavailable:") {
            StatusCode::SERVICE_UNAVAILABLE
        } else if message.contains("missing ") || message.contains("header") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self { status, message }
    }
}

fn structured_status(err: &anyhow::Error) -> Option<StatusCode> {
    err.chain().find_map(|cause| {
        cause
            .downcast_ref::<ServiceError>()
            .map(|err| match err.kind() {
                ServiceErrorKind::BadRequest => StatusCode::BAD_REQUEST,
                ServiceErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
                ServiceErrorKind::Forbidden => StatusCode::FORBIDDEN,
                ServiceErrorKind::NotFound => StatusCode::NOT_FOUND,
                ServiceErrorKind::Conflict => StatusCode::CONFLICT,
                ServiceErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            })
    })
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}
