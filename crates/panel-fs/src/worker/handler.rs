//! Handler that processes `FsWorkerRequest` messages using `ConfinedFs`.

use super::protocol::{FsWorkerRequest, FsWorkerResponse};
use crate::fs::ConfinedFs;

/// Request handler executed within the dropped-privilege worker context.
pub struct FsWorkerHandler {
    fs: ConfinedFs,
}

impl FsWorkerHandler {
    /// Creates a new handler backed by a `ConfinedFs` instance.
    pub fn new(fs: ConfinedFs) -> Self {
        Self { fs }
    }

    /// Dispatches a single `FsWorkerRequest` and returns its `FsWorkerResponse`.
    pub fn handle(&self, request: FsWorkerRequest) -> FsWorkerResponse {
        match request {
            FsWorkerRequest::Ping => FsWorkerResponse::Pong,

            FsWorkerRequest::ListDir { path, options } => {
                FsWorkerResponse::ListDir(self.fs.list_dir(path, options))
            }

            FsWorkerRequest::Stat { path } => FsWorkerResponse::Stat(self.fs.metadata(path)),

            FsWorkerRequest::ReadFile { path, max_bytes } => {
                FsWorkerResponse::ReadFile(self.fs.read_to_vec(path, max_bytes))
            }

            FsWorkerRequest::ReadRange {
                path,
                start,
                length,
            } => FsWorkerResponse::ReadRange(self.fs.read_range(path, start, length)),

            FsWorkerRequest::WriteAtomic {
                path,
                data,
                mode,
                expected_etag,
            } => FsWorkerResponse::WriteAtomic(self.fs.write_atomic_checked(
                path,
                &data,
                mode,
                expected_etag.as_deref(),
            )),

            FsWorkerRequest::CreateDir { path, recursive } => {
                let res = if recursive {
                    self.fs.create_dir_all(path)
                } else {
                    self.fs.create_dir(path)
                };
                FsWorkerResponse::CreateDir(res)
            }

            FsWorkerRequest::Remove { path, recursive } => {
                let res = if recursive {
                    self.fs.remove_dir_all(path)
                } else {
                    self.fs.remove_file(path)
                };
                FsWorkerResponse::Remove(res)
            }

            FsWorkerRequest::Rename {
                source,
                destination,
            } => FsWorkerResponse::Rename(self.fs.rename(source, destination)),

            FsWorkerRequest::Copy {
                source,
                destination,
                recursive,
            } => {
                let res = if recursive {
                    self.fs.copy_dir_all(source, destination)
                } else {
                    self.fs.copy_file(source, destination)
                };
                FsWorkerResponse::Copy(res)
            }

            FsWorkerRequest::SetPermissions { path, mode } => {
                FsWorkerResponse::SetPermissions(self.fs.set_permissions(path, mode))
            }

            FsWorkerRequest::ChmodRecursive {
                path,
                file_mode,
                dir_mode,
            } => {
                FsWorkerResponse::ChmodRecursive(self.fs.chmod_recursive(path, file_mode, dir_mode))
            }

            FsWorkerRequest::Chown {
                path,
                uid,
                gid,
                caller_uid,
                is_admin,
            } => FsWorkerResponse::Chown(self.fs.chown(path, uid, gid, caller_uid, is_admin)),

            FsWorkerRequest::Trash { path } => {
                let trash_mgr = crate::trash::TrashManager::new(&self.fs);
                FsWorkerResponse::Trash(trash_mgr.move_to_trash(&path))
            }

            FsWorkerRequest::TrashList => {
                let trash_mgr = crate::trash::TrashManager::new(&self.fs);
                FsWorkerResponse::TrashList(trash_mgr.list_trash())
            }

            FsWorkerRequest::TrashRestore { trash_id } => {
                let trash_mgr = crate::trash::TrashManager::new(&self.fs);
                FsWorkerResponse::TrashRestore(trash_mgr.restore_trash(&trash_id))
            }

            FsWorkerRequest::TrashEmpty => {
                let trash_mgr = crate::trash::TrashManager::new(&self.fs);
                FsWorkerResponse::TrashEmpty(trash_mgr.empty_trash())
            }

            FsWorkerRequest::TrashDelete { trash_id } => {
                let trash_mgr = crate::trash::TrashManager::new(&self.fs);
                FsWorkerResponse::TrashDelete(trash_mgr.delete_permanently(&trash_id))
            }

            FsWorkerRequest::WriteChunk { path, offset, data } => {
                FsWorkerResponse::WriteChunk(self.fs.write_chunk(&path, offset, &data))
            }

            FsWorkerRequest::ZipDirectory { path } => {
                FsWorkerResponse::ZipDirectory(self.fs.zip_directory(&path))
            }

            FsWorkerRequest::UploadInit {
                target_path,
                file_name,
                total_size,
            } => {
                let upload_mgr = crate::upload::UploadManager::new();
                FsWorkerResponse::UploadInit(upload_mgr.init_session(
                    &self.fs,
                    &target_path,
                    &file_name,
                    total_size,
                ))
            }

            FsWorkerRequest::UploadGetSession { upload_id } => {
                let upload_mgr = crate::upload::UploadManager::new();
                FsWorkerResponse::UploadGetSession(upload_mgr.get_session(&self.fs, &upload_id))
            }

            FsWorkerRequest::UploadAppendChunk {
                upload_id,
                offset,
                data,
            } => {
                let upload_mgr = crate::upload::UploadManager::new();
                FsWorkerResponse::UploadAppendChunk(
                    upload_mgr.append_chunk(&self.fs, &upload_id, offset, &data),
                )
            }

            FsWorkerRequest::ArchiveCompress {
                source_paths,
                destination,
                format,
            } => {
                let res = match format {
                    crate::archive::ArchiveFormat::Zip => {
                        crate::archive::compress_zip(&self.fs, &source_paths, &destination)
                    }
                    crate::archive::ArchiveFormat::TarGz => {
                        crate::archive::compress_tar_gz(&self.fs, &source_paths, &destination)
                    }
                };
                FsWorkerResponse::ArchiveCompress(res)
            }

            FsWorkerRequest::ArchiveExtract {
                archive_path,
                destination_dir,
                format,
                limits,
            } => {
                let res = crate::archive::extract_archive(
                    &self.fs,
                    &archive_path,
                    &destination_dir,
                    format,
                    limits,
                );
                FsWorkerResponse::ArchiveExtract(res)
            }

            FsWorkerRequest::Search { path, options } => {
                let res =
                    crate::search::search_confined(&self.fs, std::path::Path::new(&path), &options);
                FsWorkerResponse::Search(res)
            }

            FsWorkerRequest::Shutdown => FsWorkerResponse::ShutdownAck,
        }
    }
}
