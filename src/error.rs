use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CruiserError {
    InvalidInput(String),
    Io(std::io::Error),
    Parse(String),
}

impl Display for CruiserError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Parse(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
        }
    }
}

impl std::error::Error for CruiserError {}

impl From<std::io::Error> for CruiserError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, CruiserError>;
