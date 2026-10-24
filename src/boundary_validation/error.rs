use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CruiserError {
    InvalidInput(String),
    Io {
        context: String,
        source: std::io::Error,
    },
    Parse {
        context: String,
        source: Box<dyn std::error::Error>,
    },
}

impl CruiserError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn parse(
        context: impl Into<String>,
        source: impl std::error::Error + 'static,
    ) -> Self {
        Self::Parse {
            context: context.into(),
            source: Box::new(source),
        }
    }

    pub(crate) fn with_context(self, context: impl Into<String>) -> Self {
        let context = context.into();
        match self {
            Self::InvalidInput(message) => Self::InvalidInput(format!("{context}: {message}")),
            Self::Io {
                context: existing,
                source,
            } => Self::Io {
                context: join_context(&context, &existing),
                source,
            },
            Self::Parse {
                context: existing,
                source,
            } => Self::Parse {
                context: join_context(&context, &existing),
                source,
            },
        }
    }
}

impl Display for CruiserError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => formatter.write_str(message),
            Self::Io { context, source } if context.is_empty() => {
                write!(formatter, "I/O error: {source}")
            }
            Self::Io { context, source } => write!(formatter, "I/O error: {context}: {source}"),
            Self::Parse { context, source } if context.is_empty() => {
                formatter.write_str(&source.to_string())
            }
            Self::Parse { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

impl std::error::Error for CruiserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source.as_ref()),
            Self::InvalidInput(_) => None,
        }
    }
}

impl From<std::io::Error> for CruiserError {
    fn from(error: std::io::Error) -> Self {
        Self::io("", error)
    }
}

pub type Result<T> = std::result::Result<T, CruiserError>;

fn join_context(prefix: &str, existing: &str) -> String {
    if existing.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}: {existing}")
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::CruiserError;

    #[test]
    fn parse_errors_keep_the_original_source() {
        // Arrange
        let source = std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed input");

        // Act
        let error = CruiserError::parse("boundary policy policy.yaml", source);

        // Assert
        assert!(error.source().is_some());
        assert_eq!(
            error.to_string(),
            "boundary policy policy.yaml: malformed input"
        );
    }
}
