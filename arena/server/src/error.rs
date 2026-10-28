//! API errors: `{ "error": { "code", "message", "details" } }` with the HTTP status of TABLE_API.md section 10.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Value,
}

pub type Res<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            code,
            message: message.into(),
            details: json!({}),
        }
    }
    pub fn with(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
    pub fn bad_request(m: impl Into<String>) -> Self {
        Self::new(400, "bad_request", m)
    }
    pub fn unauthorized() -> Self {
        Self::new(401, "unauthorized", "missing or invalid bearer token")
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::new(403, "forbidden", m)
    }
    pub fn not_found(what: &str) -> Self {
        Self::new(404, "not_found", format!("{what} not found"))
    }
    pub fn conflict(code: &'static str, m: impl Into<String>) -> Self {
        Self::new(409, code, m)
    }
    pub fn internal(m: impl std::fmt::Display) -> Self {
        Self::new(500, "internal", m.to_string())
    }
    pub fn body(&self) -> Value {
        json!({"error": {"code": self.code, "message": self.message, "details": self.details}})
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(self.body())).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::internal(format!("storage: {e}"))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::internal(format!("json: {e}"))
    }
}
