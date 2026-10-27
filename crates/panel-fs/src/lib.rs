//! `panel-fs`: Path-confined file operations with strict boundary checks.
//!
//! Complies with PRD Section 5.3 and Task P5-T1:
//! - Resolves paths strictly confined beneath a designated jail directory
//! - Uses path validation as defense in depth inside a separate, jailed tenant worker
//! - Pathname validation alone is not an atomic `openat2` capability
//! - Never builds paths via string concatenation
//! - Prevents directory traversal (`..`), symlink escapes, null bytes, unicode tricks, and root deletions
//! - Provides safe operations: list (with sorting/pagination), stat, read, range read, atomic write, mkdir, rename, copy, chmod

pub mod archive;
pub mod entry;
pub mod error;
pub mod fs;
pub mod path;
pub mod permission;
pub mod policy;
pub mod search;
pub mod trash;
pub mod upload;
pub mod worker;

pub use archive::{
    ArchiveFormat, ArchiveResult, ExtractLimits, compress_tar_gz, compress_zip, extract_archive,
    sanitize_archive_entry_path,
};
pub use entry::{FsEntry, ListOptions, ListResult, SortBy, SortOrder};
pub use error::FsError;
pub use fs::ConfinedFs;
pub use path::{
    ConfinedPath, normalize_virtual_components, resolve_confined_path, validate_raw_path_str,
};
pub use permission::{
    DEFAULT_DIR_MODE, DEFAULT_FILE_MODE, PERMISSIVE_777_WARNING, PermissionGrid,
    check_mode_security_warning, format_octal_mode, parse_octal_mode,
};
pub use policy::SymlinkPolicy;
pub use search::{
    ContentMatch, DEFAULT_MAX_FILE_SIZE_FOR_CONTENT, DEFAULT_MAX_SEARCH_RESULTS,
    DEFAULT_SEARCH_TIMEOUT_SECS, SearchOptions, SearchReport, SearchResultItem, search_confined,
};
pub use trash::{TrashEmptyResult, TrashManager, TrashedItem};
pub use upload::{UPLOAD_TMP_DIR, UploadManager, UploadMetadata};
pub use worker::{
    FsWorkerHandler, FsWorkerPool, FsWorkerPoolConfig, FsWorkerRequest, FsWorkerResponse,
    TenantWorkerKey,
};
