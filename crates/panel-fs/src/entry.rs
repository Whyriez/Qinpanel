//! File system entry representations and listing query options.

use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::Path;
use std::time::UNIX_EPOCH;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Detailed metadata information for an entry inside a confined root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    /// File or directory base name (e.g., "index.html").
    pub name: String,
    /// Normalized POSIX relative path from confined root (e.g., "sites/example.com/index.html").
    pub relative_path: String,
    /// Indicates whether the entry is a directory.
    pub is_dir: bool,
    /// Indicates whether the entry is a regular file.
    pub is_file: bool,
    /// Indicates whether the entry is a symbolic link.
    pub is_symlink: bool,
    /// If symbolic link, the destination target as recorded on disk.
    pub symlink_target: Option<String>,
    /// Size in bytes (0 for directories).
    pub size_bytes: u64,
    /// Last modified time in seconds since Unix epoch.
    pub modified_at: Option<i64>,
    /// Creation time in seconds since Unix epoch.
    pub created_at: Option<i64>,
    /// POSIX file permission bits (e.g., 0o755, 0o644).
    pub mode: u32,
    /// Octal representation string (e.g., "0755", "0644").
    pub mode_octal: String,
    /// POSIX User ID (UID), if available on host.
    pub uid: Option<u32>,
    /// POSIX Group ID (GID), if available on host.
    pub gid: Option<u32>,
    /// Inferred MIME type (e.g. "text/html", "application/json").
    pub mime_type: Option<String>,
}

impl FsEntry {
    /// Constructs an `FsEntry` from filesystem metadata.
    pub fn from_metadata(
        name: String,
        relative_path: String,
        metadata: &Metadata,
        symlink_target: Option<String>,
    ) -> Self {
        let is_symlink = metadata.file_type().is_symlink();
        let is_dir = metadata.is_dir();
        let is_file = metadata.is_file();
        let size_bytes = if is_dir { 0 } else { metadata.len() };

        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        let created_at = metadata
            .created()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);

        #[cfg(unix)]
        let (mode, uid, gid) = {
            let m = metadata.permissions().mode() & 0o7777;
            (m, Some(metadata.uid()), Some(metadata.gid()))
        };

        #[cfg(not(unix))]
        let (mode, uid, gid) = {
            let m = if is_dir { 0o755 } else { 0o644 };
            (m, None, None)
        };

        let mode_octal = format!("{:04o}", mode & 0o7777);

        let mime_type = if is_file {
            mime_guess::from_path(Path::new(&name))
                .first()
                .map(|m| m.to_string())
        } else if is_dir {
            Some("inode/directory".to_string())
        } else {
            None
        };

        Self {
            name,
            relative_path,
            is_dir,
            is_file,
            is_symlink,
            symlink_target,
            size_bytes,
            modified_at,
            created_at,
            mode,
            mode_octal,
            uid,
            gid,
            mime_type,
        }
    }
}

/// Field criteria for sorting directory listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Name,
    Size,
    ModifiedAt,
    CreatedAt,
}

/// Sort direction for directory listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// Query and filtering parameters for directory listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOptions {
    /// Maximum number of items to return in a single page (default 100, max 10000).
    pub limit: usize,
    /// Number of items to skip from beginning.
    pub offset: usize,
    /// Field to sort results by.
    pub sort_by: SortBy,
    /// Sort direction.
    pub sort_order: SortOrder,
    /// Whether to include dot-prefixed hidden files (e.g. `.env`, `.git`).
    pub show_hidden: bool,
    /// Optional case-insensitive substring search query on entry names.
    pub search_query: Option<String>,
    /// Optional filter by file extension without dot (e.g., "php", "log").
    pub extension: Option<String>,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            limit: 100,
            offset: 0,
            sort_by: SortBy::Name,
            sort_order: SortOrder::Asc,
            show_hidden: false,
            search_query: None,
            extension: None,
        }
    }
}

/// Paginated result of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListResult {
    /// List of file/directory entries in this page.
    pub entries: Vec<FsEntry>,
    /// Total count of matching entries before limit and offset.
    pub total_count: usize,
    /// Offset of current page.
    pub offset: usize,
    /// Limit applied to current page.
    pub limit: usize,
    /// Indicates if there are subsequent entries available.
    pub has_more: bool,
}
