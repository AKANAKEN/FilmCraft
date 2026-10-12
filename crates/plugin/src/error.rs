//! Error types for the FilmCraft plugin system.
//!
//! Follows the "Never crash" rule: all operations return `Result<T, PluginError>`,
//! hostile dimensions or buffer bounds are validated up front, and failures never panic.

use thiserror::Error;

/// All recoverable errors produced by plugin loading, manifest parsing, or execution.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PluginError {
    #[error("plugin manifest error: {0}")]
    Manifest(String),

    #[error("plugin initialization failed for '{id}': {reason}")]
    InitializationFailed { id: String, reason: String },

    #[error("invalid frame dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },

    #[error("buffer size mismatch: expected at least {expected} bytes, got {actual}")]
    BufferSizeMismatch { expected: usize, actual: usize },

    #[error("invalid plugin parameter '{name}': {reason}")]
    InvalidParameter { name: String, reason: String },

    #[error("plugin execution failed: {0}")]
    ExecutionFailed(String),

    #[error("memory limit exceeded: allowed {limit_mb} MB, requested {requested_mb} MB")]
    MemoryLimitExceeded { limit_mb: u32, requested_mb: u32 },

    #[error("unsupported capability or feature: {0}")]
    Unsupported(String),

    #[error("plugin not found: '{0}'")]
    NotFound(String),
}
