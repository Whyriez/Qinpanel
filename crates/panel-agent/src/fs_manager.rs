//! Agent file manager subsystem coordinating `fs-worker` requests.
//!
//! Complies with PRD Section 5.3 and Task P5-T3:
//! - Maps tenant requests to their home jail and dropped UID/GID context
//! - Coordinates listing, stat, mkdir, rename, copy, trash, and delete operations
//! - Converts worker responses to typed `panel-proto` DTOs

use crate::AgentError;
use crate::system::SystemRunner;
use panel_core::Username;
use panel_fs::{
    FsEntry, FsWorkerPool, FsWorkerPoolConfig, FsWorkerRequest, FsWorkerResponse, ListOptions,
    ListResult, SortBy, SortOrder, SymlinkPolicy, TenantWorkerKey, TrashedItem,
};
use panel_proto::{
    ArchiveFormatDto, ArchiveResultDto, ContentMatchDto, FsActionResponse,
    FsArchiveCompressRequest, FsArchiveCompressResponse, FsArchiveExtractRequest,
    FsArchiveExtractResponse, FsChmodRecursiveRequest, FsChmodRecursiveResponse, FsChmodRequest,
    FsChmodResponse, FsChownRequest, FsChownResponse, FsCopyRequest, FsDeleteRequest, FsEntryDto,
    FsListRequest, FsListResponse, FsMkdirRequest, FsReadFileRequest, FsReadFileResponse,
    FsReadRangeRequest, FsReadRangeResponse, FsRenameRequest, FsSearchItemDto, FsSearchRequest,
    FsSearchResponse, FsStatRequest, FsStatResponse, FsTrashEmptyRequest, FsTrashEmptyResponse,
    FsTrashListRequest, FsTrashListResponse, FsTrashRequest, FsTrashResponse,
    FsTrashRestoreRequest, FsTrashRestoreResponse, FsUploadAppendChunkRequest,
    FsUploadAppendChunkResponse, FsUploadGetSessionRequest, FsUploadGetSessionResponse,
    FsUploadInitRequest, FsUploadInitResponse, FsWriteAtomicRequest, FsWriteAtomicResponse,
    FsWriteEditorRequest, FsZipDirectoryRequest, FsZipDirectoryResponse, TrashedItemDto,
    UploadMetadataDto,
};
use std::path::PathBuf;
use std::sync::Arc;

pub const DEFAULT_HOME_BASE: &str = "/home";

pub struct FsManager {
    runner: Arc<dyn SystemRunner>,
    pool: Arc<FsWorkerPool>,
    home_base: PathBuf,
}

impl FsManager {
    pub fn new(runner: Arc<dyn SystemRunner>) -> Self {
        let pool = Arc::new(FsWorkerPool::new(FsWorkerPoolConfig {
            idle_timeout: std::time::Duration::from_secs(60),
            max_workers: 64,
            symlink_policy: SymlinkPolicy::DenyAll,
        }));
        Self {
            runner,
            pool,
            home_base: PathBuf::from(DEFAULT_HOME_BASE),
        }
    }

    /// In-process fixture manager; never present in a release build.
    #[cfg(debug_assertions)]
    pub fn with_home_base(runner: Arc<dyn SystemRunner>, home_base: PathBuf) -> Self {
        let pool = Arc::new(FsWorkerPool::in_process_for_tests(FsWorkerPoolConfig {
            idle_timeout: std::time::Duration::from_secs(60),
            max_workers: 64,
            symlink_policy: SymlinkPolicy::DenyAll,
        }));
        Self {
            runner,
            pool,
            home_base,
        }
    }

    pub fn pool(&self) -> &Arc<FsWorkerPool> {
        &self.pool
    }

    /// Resolves tenant's home path, UID, and GID.
    pub fn get_tenant_key(&self, username_raw: &str) -> Result<TenantWorkerKey, AgentError> {
        let username =
            Username::parse(username_raw).map_err(|e| AgentError::InvalidInput(e.to_string()))?;
        let home = self.home_base.join(username.as_str());

        let output = self
            .runner
            .run_command("getent", &["passwd", username.as_str()])?;
        if !output.status.success() {
            return Err(AgentError::InvalidInput(
                "Tenant account lookup failed".into(),
            ));
        }
        let text = std::str::from_utf8(&output.stdout)
            .map_err(|_| AgentError::InvalidInput("Invalid tenant account record".into()))?;
        let (uid, gid) =
            Self::parse_passwd_identity(text, username.as_str(), &home).ok_or_else(|| {
                AgentError::InvalidInput("Tenant account identity/home mismatch".into())
            })?;
        Ok(TenantWorkerKey::new(uid, gid, home))
    }

    fn parse_passwd_identity(
        text: &str,
        username: &str,
        home: &std::path::Path,
    ) -> Option<(u32, u32)> {
        let mut records = text.trim().lines();
        let fields: Vec<_> = records.next()?.split(':').collect();
        if records.next().is_some()
            || fields.len() != 7
            || fields[0] != username
            || std::path::Path::new(fields[5]) != home
        {
            return None;
        }
        let uid = fields[2]
            .parse::<u32>()
            .ok()
            .filter(|uid| (10000..=59999).contains(uid))?;
        let gid = fields[3]
            .parse::<u32>()
            .ok()
            .filter(|gid| (10000..=59999).contains(gid))?;
        Some((uid, gid))
    }

    /// Lists directory entries with filtering and pagination.
    pub async fn list_dir(&self, req: FsListRequest) -> Result<FsListResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let sort_by = match req.sort_by.to_lowercase().as_str() {
            "size" => SortBy::Size,
            "modified" | "modified_at" => SortBy::ModifiedAt,
            "created" | "created_at" => SortBy::CreatedAt,
            _ => SortBy::Name,
        };

        let sort_order = if req.sort_order.eq_ignore_ascii_case("desc") {
            SortOrder::Desc
        } else {
            SortOrder::Asc
        };

        let options = ListOptions {
            limit: req.limit,
            offset: req.offset,
            sort_by,
            sort_order,
            show_hidden: req.show_hidden,
            search_query: req.search_query,
            extension: req.extension,
        };

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::ListDir {
                    path: req.path,
                    options,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ListDir(Ok(list)) => Ok(Self::convert_list_result(list)),
            FsWorkerResponse::ListDir(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for list_dir: {:?}",
                other
            ))),
        }
    }

    /// Retrieves metadata for a specific entry.
    pub async fn stat(&self, req: FsStatRequest) -> Result<FsStatResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(&tenant, FsWorkerRequest::Stat { path: req.path })
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Stat(Ok(entry)) => Ok(FsStatResponse {
                entry: Self::convert_entry(entry),
            }),
            FsWorkerResponse::Stat(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for stat: {:?}",
                other
            ))),
        }
    }

    /// Creates a directory.
    pub async fn mkdir(&self, req: FsMkdirRequest) -> Result<FsActionResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::CreateDir {
                    path: req.path,
                    recursive: req.recursive,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::CreateDir(Ok(())) => Ok(FsActionResponse {
                success: true,
                message: "Directory created successfully".into(),
            }),
            FsWorkerResponse::CreateDir(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for mkdir: {:?}",
                other
            ))),
        }
    }

    /// Renames or moves a file or directory.
    pub async fn rename(&self, req: FsRenameRequest) -> Result<FsActionResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::Rename {
                    source: req.source,
                    destination: req.destination,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Rename(Ok(())) => Ok(FsActionResponse {
                success: true,
                message: "Item renamed/moved successfully".into(),
            }),
            FsWorkerResponse::Rename(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for rename: {:?}",
                other
            ))),
        }
    }

    /// Copies a file or directory.
    pub async fn copy(&self, req: FsCopyRequest) -> Result<FsActionResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::Copy {
                    source: req.source,
                    destination: req.destination,
                    recursive: req.recursive,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Copy(Ok(bytes)) => Ok(FsActionResponse {
                success: true,
                message: format!("Copied {bytes} bytes successfully"),
            }),
            FsWorkerResponse::Copy(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for copy: {:?}",
                other
            ))),
        }
    }

    /// Moves a file or directory into `.panel-trash`.
    pub async fn trash(&self, req: FsTrashRequest) -> Result<FsTrashResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(&tenant, FsWorkerRequest::Trash { path: req.path })
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Trash(Ok(item)) => Ok(FsTrashResponse {
                item: Self::convert_trashed_item(item),
            }),
            FsWorkerResponse::Trash(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for trash: {:?}",
                other
            ))),
        }
    }

    /// Lists all items currently in `.panel-trash`.
    pub async fn trash_list(
        &self,
        req: FsTrashListRequest,
    ) -> Result<FsTrashListResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(&tenant, FsWorkerRequest::TrashList)
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::TrashList(Ok(items)) => Ok(FsTrashListResponse {
                items: items.into_iter().map(Self::convert_trashed_item).collect(),
            }),
            FsWorkerResponse::TrashList(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for trash_list: {:?}",
                other
            ))),
        }
    }

    /// Restores an item from `.panel-trash`.
    pub async fn trash_restore(
        &self,
        req: FsTrashRestoreRequest,
    ) -> Result<FsTrashRestoreResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::TrashRestore {
                    trash_id: req.trash_id,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::TrashRestore(Ok(item)) => Ok(FsTrashRestoreResponse {
                item: Self::convert_trashed_item(item),
            }),
            FsWorkerResponse::TrashRestore(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for trash_restore: {:?}",
                other
            ))),
        }
    }

    /// Empties `.panel-trash`, purging all items.
    pub async fn trash_empty(
        &self,
        req: FsTrashEmptyRequest,
    ) -> Result<FsTrashEmptyResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(&tenant, FsWorkerRequest::TrashEmpty)
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::TrashEmpty(Ok(result)) => Ok(FsTrashEmptyResponse {
                purged_count: result.purged_count,
                bytes_freed: result.bytes_freed,
            }),
            FsWorkerResponse::TrashEmpty(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for trash_empty: {:?}",
                other
            ))),
        }
    }

    /// Permanently deletes an item from the filesystem.
    pub async fn delete(&self, req: FsDeleteRequest) -> Result<FsActionResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::Remove {
                    path: req.path,
                    recursive: req.recursive,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Remove(Ok(())) => Ok(FsActionResponse {
                success: true,
                message: "Permanently deleted successfully".into(),
            }),
            FsWorkerResponse::Remove(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for delete: {:?}",
                other
            ))),
        }
    }

    /// Initializes a chunked resumable upload session.
    pub async fn upload_init(
        &self,
        req: FsUploadInitRequest,
    ) -> Result<FsUploadInitResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::UploadInit {
                    target_path: req.target_path,
                    file_name: req.file_name,
                    total_size: req.total_size,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::UploadInit(Ok(meta)) => Ok(FsUploadInitResponse {
                metadata: Self::convert_upload_metadata(meta),
            }),
            FsWorkerResponse::UploadInit(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for upload_init: {:?}",
                other
            ))),
        }
    }

    /// Retrieves status of an ongoing upload session.
    pub async fn upload_get_session(
        &self,
        req: FsUploadGetSessionRequest,
    ) -> Result<FsUploadGetSessionResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::UploadGetSession {
                    upload_id: req.upload_id,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::UploadGetSession(Ok(meta)) => Ok(FsUploadGetSessionResponse {
                metadata: Self::convert_upload_metadata(meta),
            }),
            FsWorkerResponse::UploadGetSession(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for upload_get_session: {:?}",
                other
            ))),
        }
    }

    /// Appends a chunk to an upload session.
    pub async fn upload_append_chunk(
        &self,
        req: FsUploadAppendChunkRequest,
    ) -> Result<FsUploadAppendChunkResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::UploadAppendChunk {
                    upload_id: req.upload_id,
                    offset: req.offset,
                    data: req.data,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::UploadAppendChunk(Ok(meta)) => Ok(FsUploadAppendChunkResponse {
                metadata: Self::convert_upload_metadata(meta),
            }),
            FsWorkerResponse::UploadAppendChunk(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for upload_append_chunk: {:?}",
                other
            ))),
        }
    }

    /// Reads a specific range of bytes from a file.
    pub async fn read_range(
        &self,
        req: FsReadRangeRequest,
    ) -> Result<FsReadRangeResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::ReadRange {
                    path: req.path,
                    start: req.start,
                    length: req.length,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ReadRange(Ok(data)) => Ok(FsReadRangeResponse { data }),
            FsWorkerResponse::ReadRange(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for read_range: {:?}",
                other
            ))),
        }
    }

    /// Archives a directory into a ZIP stream.
    pub async fn zip_directory(
        &self,
        req: FsZipDirectoryRequest,
    ) -> Result<FsZipDirectoryResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(&tenant, FsWorkerRequest::ZipDirectory { path: req.path })
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ZipDirectory(Ok(data)) => Ok(FsZipDirectoryResponse { data }),
            FsWorkerResponse::ZipDirectory(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for zip_directory: {:?}",
                other
            ))),
        }
    }

    /// Compresses files or directories into an archive.
    pub async fn archive_compress(
        &self,
        req: FsArchiveCompressRequest,
    ) -> Result<FsArchiveCompressResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let format = match req.format {
            ArchiveFormatDto::Zip => panel_fs::ArchiveFormat::Zip,
            ArchiveFormatDto::TarGz => panel_fs::ArchiveFormat::TarGz,
        };
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::ArchiveCompress {
                    source_paths: req.source_paths,
                    destination: req.destination,
                    format,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ArchiveCompress(Ok(res)) => Ok(FsArchiveCompressResponse {
                result: ArchiveResultDto {
                    entries_processed: res.entries_processed,
                    total_bytes: res.total_bytes,
                    target_path: res.target_path,
                },
            }),
            FsWorkerResponse::ArchiveCompress(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for archive_compress: {:?}",
                other
            ))),
        }
    }

    /// Safely extracts an archive file within the jail.
    pub async fn archive_extract(
        &self,
        req: FsArchiveExtractRequest,
    ) -> Result<FsArchiveExtractResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let format = req.format.map(|f| match f {
            ArchiveFormatDto::Zip => panel_fs::ArchiveFormat::Zip,
            ArchiveFormatDto::TarGz => panel_fs::ArchiveFormat::TarGz,
        });
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::ArchiveExtract {
                    archive_path: req.archive_path,
                    destination_dir: req.destination_dir,
                    format,
                    limits: None,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ArchiveExtract(Ok(res)) => Ok(FsArchiveExtractResponse {
                result: ArchiveResultDto {
                    entries_processed: res.entries_processed,
                    total_bytes: res.total_bytes,
                    target_path: res.target_path,
                },
            }),
            FsWorkerResponse::ArchiveExtract(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for archive_extract: {:?}",
                other
            ))),
        }
    }

    pub async fn chmod(&self, req: FsChmodRequest) -> Result<FsChmodResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let warning = panel_fs::check_mode_security_warning(req.mode);
        let mode_octal = panel_fs::format_octal_mode(req.mode);
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::SetPermissions {
                    path: req.path.clone(),
                    mode: req.mode,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::SetPermissions(Ok(())) => Ok(FsChmodResponse {
                path: req.path,
                mode: req.mode,
                mode_octal,
                warning,
            }),
            FsWorkerResponse::SetPermissions(Err(e)) => {
                Err(AgentError::InvalidInput(e.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for chmod: {:?}",
                other
            ))),
        }
    }

    pub async fn chmod_recursive(
        &self,
        req: FsChmodRecursiveRequest,
    ) -> Result<FsChmodRecursiveResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let warning = panel_fs::check_mode_security_warning(req.file_mode)
            .or_else(|| panel_fs::check_mode_security_warning(req.dir_mode));
        let file_mode_octal = panel_fs::format_octal_mode(req.file_mode);
        let dir_mode_octal = panel_fs::format_octal_mode(req.dir_mode);
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::ChmodRecursive {
                    path: req.path.clone(),
                    file_mode: req.file_mode,
                    dir_mode: req.dir_mode,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ChmodRecursive(Ok(affected_count)) => Ok(FsChmodRecursiveResponse {
                path: req.path,
                file_mode: req.file_mode,
                file_mode_octal,
                dir_mode: req.dir_mode,
                dir_mode_octal,
                affected_count,
                warning,
            }),
            FsWorkerResponse::ChmodRecursive(Err(e)) => {
                Err(AgentError::InvalidInput(e.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for chmod_recursive: {:?}",
                other
            ))),
        }
    }

    pub async fn chown(&self, req: FsChownRequest) -> Result<FsChownResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::Chown {
                    path: req.path.clone(),
                    uid: req.uid,
                    gid: req.gid,
                    caller_uid: req.caller_uid,
                    is_admin: req.is_admin,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Chown(Ok(())) => Ok(FsChownResponse {
                path: req.path,
                uid: req.uid,
                gid: req.gid,
            }),
            FsWorkerResponse::Chown(Err(e)) => Err(AgentError::InvalidInput(e.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for chown: {:?}",
                other
            ))),
        }
    }

    pub async fn read_file(
        &self,
        req: FsReadFileRequest,
    ) -> Result<FsReadFileResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::ReadFile {
                    path: req.path,
                    max_bytes: req.max_bytes,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::ReadFile(Ok(data)) => Ok(FsReadFileResponse { data }),
            FsWorkerResponse::ReadFile(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for read_file: {:?}",
                other
            ))),
        }
    }

    pub async fn write_atomic(
        &self,
        req: FsWriteAtomicRequest,
    ) -> Result<FsWriteAtomicResponse, AgentError> {
        self.write_with_revision(req, None).await
    }

    pub async fn write_editor(
        &self,
        req: FsWriteEditorRequest,
    ) -> Result<FsWriteAtomicResponse, AgentError> {
        let expected = req.expected_etag;
        self.write_with_revision(
            FsWriteAtomicRequest {
                username: req.username,
                path: req.path,
                data: req.data,
                mode: None,
            },
            expected,
        )
        .await
    }

    async fn write_with_revision(
        &self,
        req: FsWriteAtomicRequest,
        expected_etag: Option<String>,
    ) -> Result<FsWriteAtomicResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::WriteAtomic {
                    path: req.path,
                    data: req.data,
                    mode: req.mode,
                    expected_etag,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::WriteAtomic(Ok(bytes_written)) => {
                Ok(FsWriteAtomicResponse { bytes_written })
            }
            FsWorkerResponse::WriteAtomic(Err(err)) => {
                Err(AgentError::InvalidInput(err.to_string()))
            }
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for write_atomic: {:?}",
                other
            ))),
        }
    }

    pub async fn search(&self, req: FsSearchRequest) -> Result<FsSearchResponse, AgentError> {
        let tenant = self.get_tenant_key(&req.username)?;
        let options = panel_fs::SearchOptions {
            query: req.query,
            search_content: req.search_content,
            case_sensitive: req.case_sensitive,
            extension: req.extension,
            max_results: req
                .max_results
                .unwrap_or(panel_fs::DEFAULT_MAX_SEARCH_RESULTS),
            timeout: std::time::Duration::from_secs(
                req.timeout_secs
                    .unwrap_or(panel_fs::DEFAULT_SEARCH_TIMEOUT_SECS),
            ),
            max_file_size_for_content: panel_fs::DEFAULT_MAX_FILE_SIZE_FOR_CONTENT,
        };

        let resp = self
            .pool
            .execute(
                &tenant,
                FsWorkerRequest::Search {
                    path: req.path,
                    options,
                },
            )
            .await
            .map_err(|e| AgentError::InvalidInput(e.to_string()))?;

        match resp {
            FsWorkerResponse::Search(Ok(report)) => Ok(FsSearchResponse {
                items: report
                    .items
                    .into_iter()
                    .map(|i| FsSearchItemDto {
                        path: i.path,
                        is_dir: i.is_dir,
                        size_bytes: i.size_bytes,
                        modified_at: i.modified_at,
                        matches: i
                            .matches
                            .into_iter()
                            .map(|m| ContentMatchDto {
                                line_number: m.line_number,
                                line_text: m.line_text,
                            })
                            .collect(),
                    })
                    .collect(),
                total_scanned: report.total_scanned,
                limit_reached: report.limit_reached,
                timed_out: report.timed_out,
                duration_ms: report.duration_ms,
            }),
            FsWorkerResponse::Search(Err(err)) => Err(AgentError::InvalidInput(err.to_string())),
            other => Err(AgentError::Internal(format!(
                "Unexpected worker response for search: {:?}",
                other
            ))),
        }
    }

    fn convert_upload_metadata(m: panel_fs::UploadMetadata) -> UploadMetadataDto {
        UploadMetadataDto {
            upload_id: m.upload_id,
            file_name: m.file_name,
            target_path: m.target_path,
            total_size: m.total_size,
            current_offset: m.current_offset,
            created_at: m.created_at,
            completed: m.completed,
        }
    }

    fn convert_entry(e: FsEntry) -> FsEntryDto {
        FsEntryDto {
            name: e.name,
            relative_path: e.relative_path,
            is_dir: e.is_dir,
            is_file: e.is_file,
            is_symlink: e.is_symlink,
            symlink_target: e.symlink_target,
            size_bytes: e.size_bytes,
            modified_at: e.modified_at,
            created_at: e.created_at,
            mode: e.mode,
            mode_octal: e.mode_octal,
            uid: e.uid,
            gid: e.gid,
            mime_type: e.mime_type,
        }
    }

    fn convert_list_result(r: ListResult) -> FsListResponse {
        FsListResponse {
            entries: r.entries.into_iter().map(Self::convert_entry).collect(),
            total_count: r.total_count,
            offset: r.offset,
            limit: r.limit,
            has_more: r.has_more,
        }
    }

    fn convert_trashed_item(t: TrashedItem) -> TrashedItemDto {
        TrashedItemDto {
            trash_id: t.trash_id,
            original_path: t.original_path,
            name: t.name,
            is_dir: t.is_dir,
            size_bytes: t.size_bytes,
            trashed_at: t.trashed_at,
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::system::MockSystemRunner;

    #[test]
    fn missing_or_malformed_nss_identity_never_uses_a_fallback_uid() {
        let runner = Arc::new(MockSystemRunner::new());
        let manager = FsManager::new(runner.clone());
        assert!(manager.get_tenant_key("p_alice").is_err());
        for record in [
            "root:x:0:0::/home/p_alice:/bin/sh",
            "p_alice:x:0:10001::/home/p_alice:/bin/sh",
            "p_alice:x:10001:0::/home/p_alice:/bin/sh",
            "p_bob:x:10001:10001::/home/p_alice:/bin/sh",
            "p_alice:x:10001:10001::/home/p_bob:/bin/sh",
            "p_alice:x:10001:10001",
            "p_alice:x:10001:10001::/home/p_alice:/bin/sh\np_bob:x:10002:10002::/home/p_bob:/bin/sh",
        ] {
            runner.set_response("getent passwd p_alice", 0, record.as_bytes(), b"");
            assert!(manager.get_tenant_key("p_alice").is_err(), "{record}");
        }
        runner.set_response("getent passwd p_alice", 2, b"", b"not found");
        assert!(manager.get_tenant_key("p_alice").is_err());
        runner.set_response(
            "getent passwd p_alice",
            0,
            b"p_alice:x:10001:10001::/home/p_alice:/usr/sbin/nologin\n",
            b"",
        );
        let identity = manager.get_tenant_key("p_alice").expect("valid identity");
        assert_eq!(identity.uid, 10001);
        assert_eq!(identity.gid, 10001);
    }
}
