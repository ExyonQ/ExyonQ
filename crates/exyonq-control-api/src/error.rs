use crate::types::ControlApiError;
use ::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CtrlError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("duplicate vhost")]
    DuplicateVhost,
    #[error("precondition failed")]
    PreconditionFailed,
    #[error("request body too large")]
    BodyTooLarge,
    #[error("{0}")]
    BadRequest(String),
    #[error("vhost not found")]
    NotFound,
    #[error("reload rejected")]
    ReloadRejected {
        code: Option<String>,
        message: String,
    },
    #[error("service unavailable")]
    ServiceUnavailable(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml deserialize error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

impl CtrlError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::DuplicateVhost | Self::ReloadRejected { .. } => StatusCode::CONFLICT,
            Self::PreconditionFailed => StatusCode::PRECONDITION_FAILED,
            Self::BodyTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::BadRequest(_) | Self::Json(_) => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Io(_) | Self::TomlDe(_) | Self::TomlSer(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> String {
        match self {
            Self::DuplicateVhost => "EXY-CTRL-0001".to_owned(),
            Self::BadRequest(_) | Self::NotFound | Self::Json(_) => "EXY-CTRL-0002".to_owned(),
            Self::PreconditionFailed => "EXY-CTRL-0003".to_owned(),
            Self::Unauthorized => "EXY-CTRL-0004".to_owned(),
            Self::BodyTooLarge
            | Self::ServiceUnavailable(_)
            | Self::Io(_)
            | Self::TomlDe(_)
            | Self::TomlSer(_) => "EXY-CTRL-0005".to_owned(),
            Self::ReloadRejected { code, .. } => {
                code.clone().unwrap_or_else(|| "EXY-CTRL-0005".to_owned())
            }
        }
    }

    pub fn api_error(&self) -> ControlApiError {
        ControlApiError {
            code: self.code(),
            message: self.safe_message(),
            details: None,
        }
    }

    fn safe_message(&self) -> String {
        match self {
            Self::Unauthorized => "missing, malformed, or invalid server token".to_owned(),
            Self::DuplicateVhost => "vhost already exists".to_owned(),
            Self::PreconditionFailed => "missing or stale If-Match revision".to_owned(),
            Self::BodyTooLarge => "request body exceeds 1048576 bytes".to_owned(),
            Self::BadRequest(message) => message.clone(),
            Self::Json(_) => "malformed JSON request body".to_owned(),
            Self::NotFound => "vhost not found".to_owned(),
            Self::ReloadRejected { message, .. } => message.clone(),
            Self::ServiceUnavailable(message) => message.clone(),
            Self::Io(_) | Self::TomlDe(_) | Self::TomlSer(_) => {
                "control api internal error".to_owned()
            }
        }
    }
}
