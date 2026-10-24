//! Extractors whose rejections use the JSON error format.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::error::AppError;

/// `Json<T>` with JSON errors (400, or 413 when the body limit is hit).
pub struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(json_rejection(rejection)),
        }
    }
}

fn json_rejection(rejection: JsonRejection) -> AppError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return AppError::PayloadTooLarge("Die Anfrage ist zu groß".into());
    }
    let message = match &rejection {
        JsonRejection::MissingJsonContentType(_) => {
            "Content-Type: application/json fehlt".to_owned()
        }
        other => format!("Ungültiger JSON-Body: {}", other.body_text()),
    };
    AppError::bad_request(message)
}

/// Parses the UUID of a token or device from a path segment.
pub fn parse_uuid(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::bad_request(format!("Ungültige ID: {raw}")))
}
