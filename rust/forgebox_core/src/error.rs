use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("path escaped its allowed root: {0}")]
    PathEscape(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protobuf error: {0}")]
    Protobuf(#[from] prost::DecodeError),

    #[error("internal error: {0}")]
    Internal(String),
}

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "INVALID_ARGUMENT",
            Self::NotFound(_) => "NOT_FOUND",
            Self::AlreadyExists(_) => "ALREADY_EXISTS",
            Self::PathEscape(_) => "PATH_ESCAPE",
            Self::Database(_) => "DATABASE",
            Self::Io(_) => "IO",
            Self::Protobuf(_) => "PROTOBUF",
            Self::Internal(_) => "INTERNAL",
        }
    }
}
