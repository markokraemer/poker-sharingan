//! API errors: `{ "error": { "code", "message", "details" } }` (same envelope as docs/TABLE_API.md §3).

use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}

impl ApiError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        ApiError { status, code, message: message.into(), details: None }
    }
    pub fn with(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
    /// 400: the body is not the expected JSON shape.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "bad_request", message)
    }
    /// 422: well-formed JSON that describes an impossible / inconsistent hand.
    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(422, code, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "internal", message)
    }
    pub fn body(&self) -> Value {
        let mut e = json!({"code": self.code, "message": self.message});
        if let Some(d) = &self.details {
            e["details"] = d.clone();
        }
        json!({"error": e})
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.code, self.status, self.message)
    }
}
impl std::error::Error for ApiError {}
