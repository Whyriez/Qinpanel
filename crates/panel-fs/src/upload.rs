//! Resumable chunked upload staging and assembly.
//!
//! Complies with PRD Section 5.3 and Task P5-T4:
//! - Staged in `.panel-upload-tmp/` within tenant root jail
//! - Tracks upload sessions, offsets, total size, and target paths
//! - Validates chunk offsets and writes chunks directly to disk
//! - Atomically finalizes to target path upon completion

use crate::error::FsError;
use crate::fs::ConfinedFs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;
use uuid::Uuid;

pub const UPLOAD_TMP_DIR: &str = ".panel-upload-tmp";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadMetadata {
    pub upload_id: String,
    pub target_path: String,
    pub file_name: String,
    pub total_size: u64,
    pub current_offset: u64,
    pub created_at: i64,
    pub completed: bool,
}

#[derive(Debug, Default, Clone)]
pub struct UploadManager;

impl UploadManager {
    pub fn new() -> Self {
        Self
    }

    /// Initializes a new resumable upload session.
    pub fn init_session(
        &self,
        fs: &ConfinedFs,
        target_path: &str,
        file_name: &str,
        total_size: u64,
    ) -> Result<UploadMetadata, FsError> {
        let upload_id = Uuid::new_v4().to_string();
        let tmp_dir = PathBuf::from(UPLOAD_TMP_DIR);
        fs.create_dir_all(&tmp_dir)?;

        let now = chrono::Utc::now().timestamp();
        let meta = UploadMetadata {
            upload_id: upload_id.clone(),
            target_path: target_path.to_string(),
            file_name: file_name.to_string(),
            total_size,
            current_offset: 0,
            created_at: now,
            completed: false,
        };

        // Write metadata
        let meta_path = tmp_dir.join(format!("{}.json", upload_id));
        let meta_bytes = serde_json::to_vec_pretty(&meta).map_err(|e| {
            FsError::UploadSession(format!("Failed to serialize upload metadata: {e}"))
        })?;
        fs.write_atomic(&meta_path, &meta_bytes, None)?;

        // Initialize empty part file
        let part_path = tmp_dir.join(format!("{}.part", upload_id));
        fs.write_atomic(&part_path, &[], None)?;

        info!(
            "Initialized upload session {} for {}",
            upload_id, target_path
        );
        Ok(meta)
    }

    /// Retrieves status and current offset of an upload session.
    pub fn get_session(&self, fs: &ConfinedFs, upload_id: &str) -> Result<UploadMetadata, FsError> {
        let meta_path = PathBuf::from(UPLOAD_TMP_DIR).join(format!("{}.json", upload_id));
        let bytes = fs.read_to_vec(&meta_path, None)?;
        let meta: UploadMetadata = serde_json::from_slice(&bytes).map_err(|e| {
            FsError::UploadSession(format!("Invalid metadata for upload {upload_id}: {e}"))
        })?;
        Ok(meta)
    }

    /// Appends a chunk to an ongoing upload session.
    pub fn append_chunk(
        &self,
        fs: &ConfinedFs,
        upload_id: &str,
        offset: u64,
        data: &[u8],
    ) -> Result<UploadMetadata, FsError> {
        let mut meta = self.get_session(fs, upload_id)?;

        if meta.completed {
            return Err(FsError::UploadSession(format!(
                "Upload session {upload_id} has already been completed"
            )));
        }

        if offset != meta.current_offset {
            return Err(FsError::UploadSession(format!(
                "Offset mismatch: expected {}, got {}",
                meta.current_offset, offset
            )));
        }

        let tmp_dir = PathBuf::from(UPLOAD_TMP_DIR);
        let part_path = tmp_dir.join(format!("{}.part", upload_id));

        let new_offset = fs.write_chunk(&part_path, offset, data)?;
        meta.current_offset = new_offset;

        if meta.current_offset >= meta.total_size {
            // Ensure parent directory exists if any
            if let Some(parent) = std::path::Path::new(&meta.target_path)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
            {
                let _ = fs.create_dir_all(parent);
            }
            // Atomic finalize: rename .part to target path
            fs.rename(&part_path, &meta.target_path)?;
            meta.completed = true;

            // Remove metadata file
            let meta_path = tmp_dir.join(format!("{}.json", upload_id));
            let _ = fs.remove_file(&meta_path);

            info!(
                "Upload session {} completed and finalized to {}",
                upload_id, meta.target_path
            );
        } else {
            // Save updated metadata
            let meta_path = tmp_dir.join(format!("{}.json", upload_id));
            let meta_bytes = serde_json::to_vec_pretty(&meta).map_err(|e| {
                FsError::UploadSession(format!("Failed to serialize metadata: {e}"))
            })?;
            fs.write_atomic(&meta_path, &meta_bytes, None)?;
        }

        Ok(meta)
    }
}
