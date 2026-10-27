//! Error types for `panel-fs` path-confined operations.

use thiserror::Error;

/// Errors that can occur during confined filesystem operations.
#[derive(Debug, Clone, PartialEq, Eq, Error, serde::Serialize, serde::Deserialize)]
pub enum FsError {
    #[error("File content conflict: {path}")]
    ContentConflict { path: String },
    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Path traversal detected: {path} ({detail})")]
    PathTraversalAttempt { path: String, detail: String },

    #[error("Symlink escape detected: {path} points to {target}")]
    SymlinkEscape { path: String, target: String },

    #[error("Symlinks are forbidden by policy: {path}")]
    SymlinkForbidden { path: String },

    #[error("Symlink loop detected: {path}")]
    SymlinkLoop { path: String },

    #[error("File or directory not found: {path}")]
    NotFound { path: String },

    #[error("Target already exists: {path}")]
    AlreadyExists { path: String },

    #[error("Expected directory but found file: {path}")]
    NotADirectory { path: String },

    #[error("Expected file but found directory: {path}")]
    IsADirectory { path: String },

    #[error("Operation prohibited on root jail: {path}")]
    RootProtectionViolation { path: String },

    #[error("Permission denied on {path}: {reason}")]
    PermissionDenied { path: String, reason: String },

    #[error("Invalid permission mode: {0}")]
    InvalidMode(String),

    #[error("File size {size} bytes exceeds maximum allowed limit {max_allowed} bytes")]
    FileTooLarge { size: u64, max_allowed: u64 },

    #[error("Quota exceeded: {reason}")]
    QuotaExceeded { reason: String },

    #[error("Archive error: {0}")]
    Archive(String),

    #[error("Upload session error: {0}")]
    UploadSession(String),

    #[error("I/O error on {path}: {detail}")]
    Io { path: String, detail: String },
}

impl FsError {
    pub fn io(path: impl Into<String>, err: std::io::Error) -> Self {
        let path = path.into();
        match err.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound { path },
            std::io::ErrorKind::AlreadyExists => Self::AlreadyExists { path },
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied {
                path,
                reason: "OS permission denied".to_string(),
            },
            _ => Self::Io {
                path,
                detail: err.to_string(),
            },
        }
    }
}
