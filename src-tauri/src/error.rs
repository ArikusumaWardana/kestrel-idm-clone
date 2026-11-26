use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("Network error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("Invalid regex pattern: {0}")]
    InvalidRegex(#[from] regex::Error),

    #[error("Download not found: {0}")]
    NotFound(String),

    #[error("Source file changed on server (ETag/Last-Modified mismatch)")]
    SourceChanged,

    #[error("Expired or forbidden download link (HTTP {0})")]
    LinkExpired(u16),

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Missing external binary '{binary}': {hint}")]
    MissingBinary { binary: String, hint: String },

    #[error("Archive extraction failed: {0}")]
    ExtractFailed(String),

    #[error("Unauthorized local bridge request")]
    Unauthorized,

    #[error("{0}")]
    NotDownloadLink(String),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
}

impl From<&AppError> for ErrorPayload {
    fn from(err: &AppError) -> Self {
        let code = match err {
            AppError::Io(_) => "IO_ERROR",
            AppError::Json(_) | AppError::TomlDe(_) | AppError::TomlSer(_) => "SERIALIZATION_ERROR",
            AppError::Http(_) => "NETWORK_ERROR",
            AppError::InvalidUrl(_) => "INVALID_URL",
            AppError::InvalidRegex(_) => "INVALID_REGEX",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::SourceChanged => "SOURCE_CHANGED",
            AppError::LinkExpired(_) => "LINK_EXPIRED",
            AppError::ChecksumMismatch { .. } => "CHECKSUM_MISMATCH",
            AppError::MissingBinary { .. } => "MISSING_BINARY",
            AppError::ExtractFailed(_) => "EXTRACT_FAILED",
            AppError::Unauthorized => "UNAUTHORIZED",
            AppError::NotDownloadLink(_) => "NOT_DOWNLOAD_LINK",
            AppError::Other(_) => "INTERNAL_ERROR",
        };
        Self {
            code: code.to_string(),
            message: err.to_string(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ErrorPayload::from(self).serialize(serializer)
    }
}

pub type AppResult<T> = Result<T, AppError>;
