//! Why an R2 operation failed, as R2 reports it.

use std::fmt;

use super::body::Algorithm;

/// A failed R2 operation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    InternalError,
    EntityTooLarge,
    EntityTooSmall,
    MetadataTooLarge,
    InvalidObjectName,
    InvalidMaxKeys,
    NoSuchUpload,
    InvalidPart,
    InvalidRange,
    BadUpload,
    BadDigest {
        algorithm: Algorithm,
        provided: String,
        actual: String,
    },
    /// The device's storage failed.
    Storage(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (message, code) = match self {
            Self::Storage(error) => return formatter.write_str(error),
            Self::BadDigest {
                algorithm,
                provided,
                actual,
            } => {
                let name = algorithm.label();
                return write!(
                    formatter,
                    "The {name} checksum you specified did not match what we received.\nYou provided a {name} checksum with value: {provided}\nActual {name} was: {actual} (10037)"
                );
            }
            Self::InternalError => ("We encountered an internal error. Please try again.", 10001),
            Self::EntityTooLarge => (
                "Your proposed upload exceeds the maximum allowed object size.",
                100_100,
            ),
            Self::EntityTooSmall => (
                "Your proposed upload is smaller than the minimum allowed object size.",
                10011,
            ),
            Self::MetadataTooLarge => (
                "Your metadata headers exceed the maximum allowed metadata size.",
                10012,
            ),
            Self::InvalidObjectName => ("The specified object name is not valid.", 10020),
            Self::InvalidMaxKeys => ("MaxKeys params must be positive integer <= 1000.", 10022),
            Self::NoSuchUpload => ("The specified multipart upload does not exist.", 10024),
            Self::InvalidPart => (
                "One or more of the specified parts could not be found.",
                10025,
            ),
            Self::InvalidRange => ("The requested range is not satisfiable", 10039),
            Self::BadUpload => ("There was a problem with the multipart upload.", 10048),
        };
        write!(formatter, "{message} ({code})")
    }
}

impl std::error::Error for Failure {}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error.to_string())
    }
}
