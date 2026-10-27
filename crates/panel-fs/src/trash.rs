//! Trash lifecycle management (`.panel-trash`) with metadata indexing and retention.
//!
//! Complies with PRD Section 5.3 and Task P5-T3:
//! - Moves deleted files/directories to `.panel-trash/files`
//! - Records metadata (original path, size, deletion timestamp) in `.panel-trash/metadata.json`
//! - Counts towards tenant disk quota since it resides inside user home
//! - Supports listing, restoring, and emptying trash

use crate::error::FsError;
use crate::fs::ConfinedFs;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::{info, warn};
use uuid::Uuid;

pub const TRASH_DIR_NAME: &str = ".panel-trash";
pub const TRASH_FILES_SUBDIR: &str = ".panel-trash/files";
pub const TRASH_METADATA_FILE: &str = ".panel-trash/metadata.json";

/// Metadata record for an item residing in the trash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrashedItem {
    /// Unique identifier in the trash (e.g., "trash_1234abcd").
    pub trash_id: String,
    /// Original relative path before deletion.
    pub original_path: String,
    /// Base name of the file or directory.
    pub name: String,
    /// Indicates whether the item was a directory.
    pub is_dir: bool,
    /// Estimated size in bytes.
    pub size_bytes: u64,
    /// Timestamp when item was moved to trash (seconds since Unix epoch).
    pub trashed_at: i64,
}

/// Summary report returned after emptying the trash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrashEmptyResult {
    /// Total count of purged files and directories.
    pub purged_count: usize,
    /// Total bytes freed.
    pub bytes_freed: u64,
}

/// Trash subsystem managing item lifecycle and index persistence.
pub struct TrashManager<'a> {
    fs: &'a ConfinedFs,
}

impl<'a> TrashManager<'a> {
    pub fn new(fs: &'a ConfinedFs) -> Self {
        Self { fs }
    }

    /// Ensures that `.panel-trash/files` exists and metadata is accessible.
    fn ensure_trash_dirs(&self) -> Result<(), FsError> {
        let _ = self.fs.create_dir_all(TRASH_FILES_SUBDIR);
        Ok(())
    }

    /// Reads the trash index metadata.
    pub fn load_index(&self) -> Result<Vec<TrashedItem>, FsError> {
        self.ensure_trash_dirs()?;

        match self.fs.read_to_vec(TRASH_METADATA_FILE, None) {
            Ok(bytes) => {
                let items: Vec<TrashedItem> = serde_json::from_slice(&bytes).unwrap_or_default();
                Ok(items)
            }
            Err(FsError::NotFound { .. }) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Saves the trash index metadata atomically.
    fn save_index(&self, items: &[TrashedItem]) -> Result<(), FsError> {
        self.ensure_trash_dirs()?;
        let data = serde_json::to_vec_pretty(items).map_err(|e| FsError::Io {
            path: TRASH_METADATA_FILE.to_string(),
            detail: format!("Failed to serialize trash index: {e}"),
        })?;

        self.fs
            .write_atomic(TRASH_METADATA_FILE, &data, Some(0o600))?;
        Ok(())
    }

    /// Moves a file or directory into trash.
    pub fn move_to_trash(&self, target_rel_path: &str) -> Result<TrashedItem, FsError> {
        let clean_path = target_rel_path.trim_matches('/');
        if clean_path.is_empty() || clean_path == "." {
            return Err(FsError::RootProtectionViolation {
                path: "Cannot trash root jail".to_string(),
            });
        }

        if clean_path.starts_with(TRASH_DIR_NAME) {
            return Err(FsError::InvalidPath(
                "Cannot trash the trash directory itself".to_string(),
            ));
        }

        let meta = self.fs.metadata(clean_path)?;
        self.ensure_trash_dirs()?;

        let trash_id = format!("trash_{}", Uuid::new_v4().simple());
        let trash_file_dest = format!("{TRASH_FILES_SUBDIR}/{trash_id}");

        // Atomic move into trash files directory
        self.fs.rename(clean_path, &trash_file_dest)?;

        let trashed_item = TrashedItem {
            trash_id: trash_id.clone(),
            original_path: clean_path.to_string(),
            name: meta.name,
            is_dir: meta.is_dir,
            size_bytes: meta.size_bytes,
            trashed_at: Utc::now().timestamp(),
        };

        let mut index = self.load_index()?;
        index.push(trashed_item.clone());
        self.save_index(&index)?;

        info!(
            "Moved {} to trash as ID {}",
            clean_path, trashed_item.trash_id
        );
        Ok(trashed_item)
    }

    /// Lists all items currently in trash.
    pub fn list_trash(&self) -> Result<Vec<TrashedItem>, FsError> {
        self.load_index()
    }

    /// Restores a trashed item back to its original path.
    pub fn restore_trash(&self, trash_id: &str) -> Result<TrashedItem, FsError> {
        let mut index = self.load_index()?;
        let pos = index
            .iter()
            .position(|it| it.trash_id == trash_id)
            .ok_or_else(|| FsError::NotFound {
                path: format!("Trash ID {trash_id}"),
            })?;

        let item = index.remove(pos);
        let trash_file_src = format!("{TRASH_FILES_SUBDIR}/{trash_id}");

        // Recreate parent directory if it was removed
        if let Some(parent) = Path::new(&item.original_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() && parent_str != "." {
                let _ = self.fs.create_dir_all(&parent_str);
            }
        }

        // Determine destination path: if original exists, append timestamp suffix
        let mut target_dest = item.original_path.clone();
        if self.fs.metadata(&target_dest).is_ok() {
            let ext = Path::new(&item.original_path)
                .extension()
                .and_then(|s| s.to_str());
            let stem = Path::new(&item.original_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file");

            let now = Utc::now().timestamp();
            target_dest = if let Some(e) = ext {
                format!("{stem}_restored_{now}.{e}")
            } else {
                format!("{stem}_restored_{now}")
            };
            warn!(
                "Original path {} exists; restoring as {}",
                item.original_path, target_dest
            );
        }

        // Move back from trash
        self.fs.rename(&trash_file_src, &target_dest)?;

        // Update index
        self.save_index(&index)?;

        info!("Restored trash item {trash_id} to {target_dest}");
        Ok(item)
    }

    /// Permanently deletes a single item from the trash.
    pub fn delete_permanently(&self, trash_id: &str) -> Result<(), FsError> {
        let mut index = self.load_index()?;
        let pos = index
            .iter()
            .position(|it| it.trash_id == trash_id)
            .ok_or_else(|| FsError::NotFound {
                path: format!("Trash ID {trash_id}"),
            })?;

        let item = index.remove(pos);
        let trash_file_src = format!("{TRASH_FILES_SUBDIR}/{trash_id}");

        if item.is_dir {
            let _ = self.fs.remove_dir_all(&trash_file_src);
        } else {
            let _ = self.fs.remove_file(&trash_file_src);
        }

        self.save_index(&index)?;
        Ok(())
    }

    /// Empties the entire trash, purging all items permanently.
    pub fn empty_trash(&self) -> Result<TrashEmptyResult, FsError> {
        let index = self.load_index()?;
        let mut bytes_freed = 0;
        let purged_count = index.len();

        for it in &index {
            bytes_freed += it.size_bytes;
        }

        // Remove files subdirectory and recreate empty
        let _ = self.fs.remove_dir_all(TRASH_FILES_SUBDIR);
        self.ensure_trash_dirs()?;

        // Reset index
        self.save_index(&[])?;

        info!("Emptied trash: {purged_count} items purged, {bytes_freed} bytes freed");
        Ok(TrashEmptyResult {
            purged_count,
            bytes_freed,
        })
    }
}
