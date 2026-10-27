//! Protocol message definitions for `fs-worker` IPC communication.

use crate::entry::{FsEntry, ListOptions, ListResult};
use crate::error::FsError;
use serde::{Deserialize, Serialize};

/// Request sent from the agent to an `fs-worker` instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "params")]
pub enum FsWorkerRequest {
    /// Liveness check.
    Ping,

    /// List directory contents.
    ListDir { path: String, options: ListOptions },

    /// Get entry metadata.
    Stat { path: String },

    /// Read an entire file into memory (with optional size limit).
    ReadFile {
        path: String,
        max_bytes: Option<u64>,
    },

    /// Read a specific byte range of a file.
    ReadRange {
        path: String,
        start: u64,
        length: u64,
    },

    /// Atomically write data to a file.
    WriteAtomic {
        path: String,
        data: Vec<u8>,
        mode: Option<u32>,
        #[serde(default)]
        expected_etag: Option<String>,
    },

    /// Create a single directory or recursive directory hierarchy.
    CreateDir { path: String, recursive: bool },

    /// Remove a file or directory.
    Remove { path: String, recursive: bool },

    /// Rename or move a file/directory.
    Rename { source: String, destination: String },

    /// Copy a file or directory.
    Copy {
        source: String,
        destination: String,
        recursive: bool,
    },

    /// Set permission mode bits on a single entry.
    SetPermissions { path: String, mode: u32 },

    /// Recursively update permissions with distinct file and directory modes.
    ChmodRecursive {
        path: String,
        file_mode: u32,
        dir_mode: u32,
    },

    /// Change ownership of a file or directory with tenant boundaries.
    Chown {
        path: String,
        uid: Option<u32>,
        gid: Option<u32>,
        caller_uid: u32,
        is_admin: bool,
    },

    /// Move a file or directory to `.panel-trash`.
    Trash { path: String },

    /// List all items currently residing in `.panel-trash`.
    TrashList,

    /// Restore an item from `.panel-trash` back to its original path.
    TrashRestore { trash_id: String },

    /// Purge all contents of `.panel-trash` permanently.
    TrashEmpty,

    /// Permanently delete a single item from `.panel-trash`.
    TrashDelete { trash_id: String },

    /// Write a chunk of data at a specific offset.
    WriteChunk {
        path: String,
        offset: u64,
        data: Vec<u8>,
    },

    /// Create a ZIP archive of a directory and return the binary payload.
    ZipDirectory { path: String },

    /// Initialize a chunked resumable upload session.
    UploadInit {
        target_path: String,
        file_name: String,
        total_size: u64,
    },

    /// Retrieve metadata and current offset of an upload session.
    UploadGetSession { upload_id: String },

    /// Append a chunk of data to an ongoing upload session.
    UploadAppendChunk {
        upload_id: String,
        offset: u64,
        data: Vec<u8>,
    },

    /// Compress source paths into an archive file.
    ArchiveCompress {
        source_paths: Vec<String>,
        destination: String,
        format: crate::archive::ArchiveFormat,
    },

    /// Safely extract an archive file to a destination directory.
    ArchiveExtract {
        archive_path: String,
        destination_dir: String,
        format: Option<crate::archive::ArchiveFormat>,
        limits: Option<crate::archive::ExtractLimits>,
    },

    /// Search files by name or content within confined boundaries.
    Search {
        path: String,
        options: crate::search::SearchOptions,
    },

    /// Graceful worker shutdown request.
    Shutdown,
}

/// Response returned from an `fs-worker` instance to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "result")]
pub enum FsWorkerResponse {
    /// Ping acknowledgment.
    Pong,

    /// Directory listing result.
    ListDir(Result<ListResult, FsError>),

    /// Stat metadata result.
    Stat(Result<FsEntry, FsError>),

    /// Read file content result.
    ReadFile(Result<Vec<u8>, FsError>),

    /// Read range result.
    ReadRange(Result<Vec<u8>, FsError>),

    /// Written bytes result.
    WriteAtomic(Result<u64, FsError>),

    /// Directory creation result.
    CreateDir(Result<(), FsError>),

    /// Removal result.
    Remove(Result<(), FsError>),

    /// Rename result.
    Rename(Result<(), FsError>),

    /// Copy result (bytes copied).
    Copy(Result<u64, FsError>),

    /// Permission setting result.
    SetPermissions(Result<(), FsError>),

    /// Recursive chmod result (count of modified items).
    ChmodRecursive(Result<usize, FsError>),

    /// Ownership changing result.
    Chown(Result<(), FsError>),

    /// Trash move result.
    Trash(Result<crate::trash::TrashedItem, FsError>),

    /// List trash result.
    TrashList(Result<Vec<crate::trash::TrashedItem>, FsError>),

    /// Restore trash result.
    TrashRestore(Result<crate::trash::TrashedItem, FsError>),

    /// Empty trash result.
    TrashEmpty(Result<crate::trash::TrashEmptyResult, FsError>),

    /// Delete single trash item result.
    TrashDelete(Result<(), FsError>),

    /// Written chunk result (new offset).
    WriteChunk(Result<u64, FsError>),

    /// Zip directory result (binary zip payload).
    ZipDirectory(Result<Vec<u8>, FsError>),

    /// Upload init result.
    UploadInit(Result<crate::upload::UploadMetadata, FsError>),

    /// Upload get session result.
    UploadGetSession(Result<crate::upload::UploadMetadata, FsError>),

    /// Upload append chunk result.
    UploadAppendChunk(Result<crate::upload::UploadMetadata, FsError>),

    /// Archive compression result.
    ArchiveCompress(Result<crate::archive::ArchiveResult, FsError>),

    /// Archive extraction result.
    ArchiveExtract(Result<crate::archive::ArchiveResult, FsError>),

    /// Search report result.
    Search(Result<crate::search::SearchReport, FsError>),

    /// Shutdown acknowledgment.
    ShutdownAck,

    /// Unexpected internal or protocol error.
    InternalError(String),
}
