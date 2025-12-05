use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Unauthorized,
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::BadRequest(msg) => write!(f, "bad request: {msg}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        if status.is_server_error() {
            tracing::error!(%status, error = %self);
        } else {
            tracing::warn!(%status, error = %self);
        }

        (
            status,
            Json(ErrorBody {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}

impl From<turso::Error> for Error {
    fn from(e: turso::Error) -> Self {
        match e {
            turso::Error::QueryReturnedNoRows => Error::NotFound("not found".into()),
            other => Error::Internal(other.to_string()),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Internal(e.to_string())
    }
}
