use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CruiserError {
    InvalidInput(String),
    Io {
        context: String,
        source: std::io::Error,
    },
    Parse(String),
}

impl CruiserError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

impl Display for CruiserError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Parse(message) => formatter.write_str(message),
            Self::Io { context, source } if context.is_empty() => {
                write!(formatter, "I/O error: {source}")
            }
            Self::Io { context, source } => write!(formatter, "I/O error: {context}: {source}"),
        }
    }
}

impl std::error::Error for CruiserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidInput(_) | Self::Parse(_) => None,
        }
    }
}

impl From<std::io::Error> for CruiserError {
    fn from(error: std::io::Error) -> Self {
        Self::io("", error)
    }
}

pub type Result<T> = std::result::Result<T, CruiserError>;
