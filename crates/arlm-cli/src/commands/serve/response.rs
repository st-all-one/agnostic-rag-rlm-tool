use serde::Serialize;

/// Generic JSON API envelope returned by route handlers.
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            status: "ok".to_string(),
            data: Some(data),
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            data: None,
            error: Some(msg.into()),
        }
    }
}

impl<T: Serialize> axum::response::IntoResponse for ApiResponse<T> {
    fn into_response(self) -> axum::response::Response {
        let status = if self.status == "ok" {
            axum::http::StatusCode::OK
        } else {
            axum::http::StatusCode::BAD_REQUEST
        };
        (status, axum::Json(self)).into_response()
    }
}
