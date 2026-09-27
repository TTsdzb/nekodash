use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

/// Stable categories for localized UI messages and recovery decisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    InvalidInput,
    Unauthorized,
    Unsupported,
    Timeout,
    Transport,
    Http,
    Decode,
    ResponseTooLarge,
    Cancelled,
    Lagged,
    Storage,
}

/// Contains a redacted description, never a request object or credential URL.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{operation}: {message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub operation: &'static str,
    pub status: Option<u16>,
    pub message: String,
}

impl Error {
    pub(crate) fn new(
        kind: ErrorKind,
        operation: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            operation,
            status: None,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, "validate input", message)
    }

    pub(crate) fn decode(operation: &'static str, error: serde_json::Error) -> Self {
        // Serde's detailed message can contain server-supplied secrets. Keep location/category.
        Self::new(
            ErrorKind::Decode,
            operation,
            format!(
                "invalid JSON schema at line {}, column {} ({:?})",
                error.line(),
                error.column(),
                error.classify()
            ),
        )
    }

    pub(crate) fn transport(operation: &'static str, error: &reqwest::Error) -> Self {
        let (kind, message) = if error.is_timeout() {
            (ErrorKind::Timeout, "request timed out")
        } else if error.is_connect() {
            (ErrorKind::Transport, "connection or TLS negotiation failed")
        } else {
            (ErrorKind::Transport, "HTTP transport failed")
        };
        Self::new(kind, operation, message)
    }

    pub(crate) fn io(operation: &'static str, error: &std::io::Error) -> Self {
        Self::new(
            ErrorKind::Storage,
            operation,
            format!("filesystem error ({:?})", error.kind()),
        )
    }

    pub(crate) fn http(operation: &'static str, status: u16, message: impl fmt::Display) -> Self {
        let kind = match status {
            401 | 403 => ErrorKind::Unauthorized,
            404 | 405 | 501 => ErrorKind::Unsupported,
            _ => ErrorKind::Http,
        };
        Self {
            kind,
            operation,
            status: Some(status),
            message: message.to_string(),
        }
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "session", "operation cancelled")
    }
}
