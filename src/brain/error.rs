use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BrainError {
    #[error("HTTP request error: {0}")]
    Request(#[from] reqwest::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("LLM service error: {0}")]
    Llm(String),

    #[error("Embedding generation failed: {0}")]
    Embedding(String),

    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    #[error("Planning error: {0}")]
    Planning(String),

    #[error("No sources found for query")]
    NoSourcesFound,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("VardaDB error: {0}")]
    Varda(String),
}

impl IntoResponse for BrainError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            BrainError::Request(ref e) => {
                tracing::error!("HTTP request error: {:?}", e);
                (StatusCode::BAD_GATEWAY, "External service error")
            }
            BrainError::Json(ref e) => {
                tracing::error!("JSON error: {:?}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Data processing error")
            }
            BrainError::Llm(ref msg) => {
                tracing::error!("LLM error: {}", msg);
                (StatusCode::BAD_GATEWAY, "AI service error")
            }
            BrainError::Embedding(ref msg) => {
                tracing::error!("Embedding error: {}", msg);
                (StatusCode::BAD_GATEWAY, "Embedding service error")
            }
            BrainError::InvalidQuery(ref msg) => {
                tracing::warn!("Invalid query: {}", msg);
                (StatusCode::BAD_REQUEST, msg.as_str())
            }
            BrainError::Planning(ref msg) => {
                tracing::warn!("Planning error: {}", msg);
                (StatusCode::BAD_GATEWAY, "Planning service error")
            }
            BrainError::NoSourcesFound => {
                tracing::info!("No sources found");
                (StatusCode::NOT_FOUND, "No relevant passages found")
            }
            BrainError::Internal(ref msg) => {
                tracing::error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
            BrainError::Varda(ref msg) => {
                tracing::error!("VardaDB error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error")
            }
        };

        let body = Json(json!({
            "error": error_message,
            "details": self.to_string(),
        }));

        (status, body).into_response()
    }
}
