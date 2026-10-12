//! Error types for the FilmCraft Plugin SDK.

use thiserror::Error;

/// All recoverable errors generated within guest plugin code.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SdkError {
    #[error("invalid parameter '{name}': {reason}")]
    InvalidParameter { name: String, reason: String },

    #[error("frame processing failed: {0}")]
    ProcessingFailed(String),

    #[error("internal guest error: {0}")]
    Internal(String),
}
