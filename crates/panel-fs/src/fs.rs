//! Core `ConfinedFs` engine performing path-confined filesystem operations.

use crate::entry::{FsEntry, ListOptions, ListResult, SortBy, SortOrder};
use crate::error::FsError;
use crate::path::{ConfinedPath, resolve_confined_path};
use crate::policy::SymlinkPolicy;
use std::cmp::Ordering;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tracing::warn;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Path-confined filesystem operations engine.
///
/// Validates paths against `root`. Production callers must also use the jailed,
/// unprivileged worker process; pathname checks alone are not a race-proof boundary.
#[derive(Debug, Clone)]
pub struct ConfinedFs {
    root: PathBuf,
    symlink_policy: SymlinkPolicy,
}

impl ConfinedFs {
    /// Creates a new `ConfinedFs` jail anchored at `root`.
    ///
    /// The root path is canonicalized immediately and verified to be an existing directory.
    pub fn new(root: impl AsRef<Path>, symlink_policy: SymlinkPolicy) -> Result<Self, FsError> {
        let root_ref = root.as_ref();
        let canonical_root = root_ref.canonicalize().map_err(|e| {
            FsError::io(
                format!("Root path {}", root_ref.display()),
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Failed to canonicalize root {}: {e}", root_ref.display()),
                ),
            )
        })?;

        if !canonical_root.is_dir() {
            return Err(FsError::NotADirectory {
                path: canonical_root.to_string_lossy().to_string(),
            });
        }

        Ok(Self {
            root: canonical_root,
            symlink_policy,
        })
    }

    /// Returns the canonical path of the root jail.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the current symlink resolution policy.
    pub fn symlink_policy(&self) -> SymlinkPolicy {
        self.symlink_policy
    }

    /// Resolves and validates an untrusted path against this confined filesystem root.
    pub fn resolve(&self, path: impl AsRef<Path>) -> Result<ConfinedPath, FsError> {
        resolve_confined_path(&self.root, path.as_ref(), self.symlink_policy)
    }

    /// Lists directory entries with filtering, sorting, and pagination.
    pub fn list_dir(
        &self,
        path: impl AsRef<Path>,
        options: ListOptions,
    ) -> Result<ListResult, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        if !abs_path.exists() {
            return Err(FsError::NotFound {
                path: confined.relative_posix(),
            });
        }

        if !abs_path.is_dir() {
            return Err(FsError::NotADirectory {
                path: confined.relative_posix(),
            });
        }

        let read_dir =
            fs::read_dir(abs_path).map_err(|e| FsError::io(confined.relative_posix(), e))?;

        let mut entries = Vec::new();

        for item_res in read_dir {
            let item = item_res.map_err(|e| FsError::io(confined.relative_posix(), e))?;

            let name = item.file_name().to_string_lossy().to_string();

            // Filter hidden files
            if !options.show_hidden && name.starts_with('.') {
                continue;
            }

            // Filter search query
            if let Some(ref query) = options.search_query
                && !name.to_lowercase().contains(&query.to_lowercase())
            {
                continue;
            }

            // Filter extension
            if let Some(ref ext) = options.extension {
                let item_ext = Path::new(&name)
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                if !item_ext.eq_ignore_ascii_case(ext) {
                    continue;
                }
            }

            let meta = item
                .metadata()
                .map_err(|e| FsError::io(format!("{}/{}", confined.relative_posix(), name), e))?;

            let is_symlink = item.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            let symlink_target = if is_symlink {
                fs::read_link(item.path())
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            } else {
                None
            };

            let entry_rel = if confined.is_root() {
                name.clone()
            } else {
                format!("{}/{}", confined.relative_posix(), name)
            };

            entries.push(FsEntry::from_metadata(
                name,
                entry_rel,
                &meta,
                symlink_target,
            ));
        }

        // Sort entries: directories first, then sort by field
        entries.sort_by(|a, b| {
            // Directories always first
            if a.is_dir && !b.is_dir {
                return Ordering::Less;
            }
            if !a.is_dir && b.is_dir {
                return Ordering::Greater;
            }

            let ord = match options.sort_by {
                SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortBy::Size => a.size_bytes.cmp(&b.size_bytes),
                SortBy::ModifiedAt => a.modified_at.cmp(&b.modified_at),
                SortBy::CreatedAt => a.created_at.cmp(&b.created_at),
            };

            match options.sort_order {
                SortOrder::Asc => ord,
                SortOrder::Desc => ord.reverse(),
            }
        });

        let total_count = entries.len();
        let limit = options.limit.clamp(1, 10000);
        let offset = options.offset;

        let paged_entries = if offset >= total_count {
            Vec::new()
        } else {
            let end = (offset + limit).min(total_count);
            entries[offset..end].to_vec()
        };

        let has_more = offset + paged_entries.len() < total_count;

        Ok(ListResult {
            entries: paged_entries,
            total_count,
            offset,
            limit,
            has_more,
        })
    }

    /// Retrieves detailed metadata for a file or directory.
    pub fn metadata(&self, path: impl AsRef<Path>) -> Result<FsEntry, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        let symlink_meta = fs::symlink_metadata(abs_path)
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        let is_symlink = symlink_meta.file_type().is_symlink();
        let symlink_target = if is_symlink {
            fs::read_link(abs_path)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        let file_name = abs_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        Ok(FsEntry::from_metadata(
            file_name,
            confined.relative_posix(),
            &symlink_meta,
            symlink_target,
        ))
    }

    /// Reads an entire file into memory with optional maximum byte limit.
    pub fn read_to_vec(
        &self,
        path: impl AsRef<Path>,
        max_bytes: Option<u64>,
    ) -> Result<Vec<u8>, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        if !abs_path.exists() {
            return Err(FsError::NotFound {
                path: confined.relative_posix(),
            });
        }

        if abs_path.is_dir() {
            return Err(FsError::IsADirectory {
                path: confined.relative_posix(),
            });
        }

        let meta = fs::metadata(abs_path).map_err(|e| FsError::io(confined.relative_posix(), e))?;

        if let Some(max) = max_bytes
            && meta.len() > max
        {
            return Err(FsError::FileTooLarge {
                size: meta.len(),
                max_allowed: max,
            });
        }

        let mut file =
            File::open(abs_path).map_err(|e| FsError::io(confined.relative_posix(), e))?;

        let mut buffer = Vec::new();
        if let Some(max) = max_bytes {
            file.take(max)
                .read_to_end(&mut buffer)
                .map_err(|e| FsError::io(confined.relative_posix(), e))?;
        } else {
            file.read_to_end(&mut buffer)
                .map_err(|e| FsError::io(confined.relative_posix(), e))?;
        }

        Ok(buffer)
    }

    /// Reads a specified range of bytes from a file (useful for HTTP range requests).
    pub fn read_range(
        &self,
        path: impl AsRef<Path>,
        start: u64,
        length: u64,
    ) -> Result<Vec<u8>, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        if !abs_path.exists() {
            return Err(FsError::NotFound {
                path: confined.relative_posix(),
            });
        }

        if abs_path.is_dir() {
            return Err(FsError::IsADirectory {
                path: confined.relative_posix(),
            });
        }

        let mut file =
            File::open(abs_path).map_err(|e| FsError::io(confined.relative_posix(), e))?;

        file.seek(SeekFrom::Start(start))
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        let mut buffer = Vec::new();
        file.take(length)
            .read_to_end(&mut buffer)
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        Ok(buffer)
    }

    /// Writes data to a file atomically via sibling temp file and atomic rename.
    ///
    /// Ensures files are never half-written or corrupted in the event of failure.
    pub fn write_atomic(
        &self,
        path: impl AsRef<Path>,
        data: &[u8],
        mode: Option<u32>,
    ) -> Result<u64, FsError> {
        self.write_atomic_checked(path, data, mode, None)
    }

    /// Conditional editor writes are serialized within the tenant worker. External writers
    /// do not share this queue; this does not claim a filesystem-wide atomic compare/swap.
    pub fn write_atomic_checked(
        &self,
        path: impl AsRef<Path>,
        data: &[u8],
        mode: Option<u32>,
        expected_etag: Option<&str>,
    ) -> Result<u64, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        let parent_dir = abs_path.parent().ok_or_else(|| {
            FsError::InvalidPath("Target path has no valid parent directory".to_string())
        })?;

        if !parent_dir.exists() {
            return Err(FsError::NotFound {
                path: parent_dir.to_string_lossy().to_string(),
            });
        }

        let file_stem = abs_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());

        let temp_filename = format!(".{file_stem}.{}.tmp", Uuid::new_v4());
        let temp_path = parent_dir.join(&temp_filename);

        #[cfg(unix)]
        let target_mode = match mode {
            Some(value) => value & 0o777,
            None => match fs::metadata(abs_path) {
                Ok(metadata) => metadata.permissions().mode() & 0o777,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0o644,
                Err(error) => return Err(FsError::io(confined.relative_posix(), error)),
            },
        };
        // Temporary contents are private until the complete file is ready to publish.
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut temp_file = options
            .open(&temp_path)
            .map_err(|e| FsError::io(temp_path.to_string_lossy().to_string(), e))?;

        if let Err(e) = temp_file.write_all(data) {
            let _ = fs::remove_file(&temp_path);
            return Err(FsError::io(confined.relative_posix(), e));
        }

        if let Err(e) = temp_file.sync_all() {
            let _ = fs::remove_file(&temp_path);
            return Err(FsError::io(confined.relative_posix(), e));
        }

        if let Some(expected) = expected_etag {
            // Probe one byte past the limit so a concurrently growing file cannot
            // match a revision computed from a silently truncated prefix.
            let current = self.read_to_vec(
                confined.relative(),
                Some(panel_core::file_revision::MAX_EDITOR_CONTENT + 1),
            );
            if !matches!(current, Ok(ref bytes) if bytes.len() as u64 <= panel_core::file_revision::MAX_EDITOR_CONTENT && panel_core::file_revision::content_etag(bytes) == expected)
            {
                drop(temp_file);
                let _ = fs::remove_file(&temp_path);
                return Err(FsError::ContentConflict {
                    path: confined.relative_posix(),
                });
            }
        }

        #[cfg(unix)]
        if let Err(error) = temp_file.set_permissions(fs::Permissions::from_mode(target_mode)) {
            let _ = fs::remove_file(&temp_path);
            return Err(FsError::io(confined.relative_posix(), error));
        }
        #[cfg(not(unix))]
        let _ = mode;
        drop(temp_file);

        // Atomic swap into place
        if let Err(e) = fs::rename(&temp_path, abs_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(FsError::io(confined.relative_posix(), e));
        }

        Ok(data.len() as u64)
    }

    /// Creates a single directory within the confined root.
    pub fn create_dir(&self, path: impl AsRef<Path>) -> Result<(), FsError> {
        let confined = self.resolve(path)?;
        let abs = confined.resolved_absolute();

        if abs.exists() {
            return Err(FsError::AlreadyExists {
                path: confined.relative_posix(),
            });
        }

        fs::create_dir(abs).map_err(|e| FsError::io(confined.relative_posix(), e))
    }

    /// Recursively creates directories along the path within the confined root.
    pub fn create_dir_all(&self, path: impl AsRef<Path>) -> Result<(), FsError> {
        let confined = self.resolve(path)?;
        let abs = confined.resolved_absolute();

        fs::create_dir_all(abs).map_err(|e| FsError::io(confined.relative_posix(), e))
    }

    /// Removes a file within the confined root.
    pub fn remove_file(&self, path: impl AsRef<Path>) -> Result<(), FsError> {
        let confined = self.resolve(path)?;
        if confined.is_root() {
            return Err(FsError::RootProtectionViolation {
                path: "Cannot remove root jail".to_string(),
            });
        }

        let abs = confined.resolved_absolute();
        if !abs.exists() {
            return Err(FsError::NotFound {
                path: confined.relative_posix(),
            });
        }

        if abs.is_dir() {
            return Err(FsError::IsADirectory {
                path: confined.relative_posix(),
            });
        }

        fs::remove_file(abs).map_err(|e| FsError::io(confined.relative_posix(), e))
    }

    /// Removes a directory and all its contents within the confined root.
    ///
    /// Strictly protects against removing the confined root jail itself.
    pub fn remove_dir_all(&self, path: impl AsRef<Path>) -> Result<(), FsError> {
        let confined = self.resolve(path)?;
        if confined.is_root() {
            return Err(FsError::RootProtectionViolation {
                path: "Cannot recursively remove root jail directory".to_string(),
            });
        }

        let abs = confined.resolved_absolute();
        if !abs.exists() {
            return Err(FsError::NotFound {
                path: confined.relative_posix(),
            });
        }

        if !abs.is_dir() {
            return Err(FsError::NotADirectory {
                path: confined.relative_posix(),
            });
        }

        fs::remove_dir_all(abs).map_err(|e| FsError::io(confined.relative_posix(), e))
    }

    /// Renames or moves a file or directory. Both source and destination must reside within root.
    pub fn rename(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<(), FsError> {
        let src_conf = self.resolve(source)?;
        let dst_conf = self.resolve(destination)?;

        if src_conf.is_root() {
            return Err(FsError::RootProtectionViolation {
                path: "Cannot rename root jail".to_string(),
            });
        }
        if dst_conf.is_root() {
            return Err(FsError::RootProtectionViolation {
                path: "Cannot overwrite root jail".to_string(),
            });
        }

        let src_abs = src_conf.resolved_absolute();
        let dst_abs = dst_conf.resolved_absolute();

        if !src_abs.exists() {
            return Err(FsError::NotFound {
                path: src_conf.relative_posix(),
            });
        }

        fs::rename(src_abs, dst_abs).map_err(|e| {
            FsError::io(
                format!(
                    "rename from {} to {}",
                    src_conf.relative_posix(),
                    dst_conf.relative_posix()
                ),
                e,
            )
        })
    }

    /// Copies a single file from source to destination.
    pub fn copy_file(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<u64, FsError> {
        let src_conf = self.resolve(source)?;
        let dst_conf = self.resolve(destination)?;

        if src_conf.is_root() || dst_conf.is_root() {
            return Err(FsError::RootProtectionViolation {
                path: "Operation prohibited on root jail".to_string(),
            });
        }

        let src_abs = src_conf.resolved_absolute();
        let dst_abs = dst_conf.resolved_absolute();

        if !src_abs.exists() {
            return Err(FsError::NotFound {
                path: src_conf.relative_posix(),
            });
        }

        if src_abs.is_dir() {
            return Err(FsError::IsADirectory {
                path: src_conf.relative_posix(),
            });
        }

        fs::copy(src_abs, dst_abs).map_err(|e| {
            FsError::io(
                format!(
                    "copy from {} to {}",
                    src_conf.relative_posix(),
                    dst_conf.relative_posix()
                ),
                e,
            )
        })
    }

    /// Recursively copies a directory tree from source to destination within root.
    pub fn copy_dir_all(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<u64, FsError> {
        let src_conf = self.resolve(source)?;
        let dst_conf = self.resolve(destination)?;

        if src_conf.is_root() || dst_conf.is_root() {
            return Err(FsError::RootProtectionViolation {
                path: "Operation prohibited on root jail".to_string(),
            });
        }

        let src_abs = src_conf.resolved_absolute();
        let dst_abs = dst_conf.resolved_absolute();

        if !src_abs.is_dir() {
            return Err(FsError::NotADirectory {
                path: src_conf.relative_posix(),
            });
        }

        self.copy_recursive_internal(src_abs, dst_abs)
    }

    fn copy_recursive_internal(&self, src: &Path, dst: &Path) -> Result<u64, FsError> {
        fs::create_dir_all(dst).map_err(|e| FsError::io(dst.to_string_lossy().to_string(), e))?;

        let mut total_bytes = 0;
        let read_dir =
            fs::read_dir(src).map_err(|e| FsError::io(src.to_string_lossy().to_string(), e))?;

        for entry in read_dir {
            let entry = entry.map_err(|e| FsError::io(src.to_string_lossy().to_string(), e))?;
            let entry_path = entry.path();
            let dest_path = dst.join(entry.file_name());

            let file_type = entry
                .file_type()
                .map_err(|e| FsError::io(entry_path.to_string_lossy().to_string(), e))?;

            if file_type.is_dir() {
                total_bytes += self.copy_recursive_internal(&entry_path, &dest_path)?;
            } else if file_type.is_file() {
                let bytes = fs::copy(&entry_path, &dest_path)
                    .map_err(|e| FsError::io(dest_path.to_string_lossy().to_string(), e))?;
                total_bytes += bytes;
            } else if file_type.is_symlink() {
                // If symlinks are resolved beneath, duplicate symlink only if allowed
                if self.symlink_policy.is_deny_all() {
                    return Err(FsError::SymlinkForbidden {
                        path: entry_path.to_string_lossy().to_string(),
                    });
                }
                #[cfg(unix)]
                {
                    let target = fs::read_link(&entry_path)
                        .map_err(|e| FsError::io(entry_path.to_string_lossy().to_string(), e))?;
                    std::os::unix::fs::symlink(&target, &dest_path)
                        .map_err(|e| FsError::io(dest_path.to_string_lossy().to_string(), e))?;
                }
            }
        }

        Ok(total_bytes)
    }

    /// Sets POSIX permission mode bits (e.g. 0o644, 0o755).
    ///
    /// Warns if an overly permissive mode (such as 0o777) is supplied.
    pub fn set_permissions(&self, path: impl AsRef<Path>, mode: u32) -> Result<(), FsError> {
        let valid_mode = mode & 0o7777;
        if mode > 0o7777 {
            return Err(FsError::InvalidMode(format!(
                "Permission mode {mode:#o} contains invalid high bits"
            )));
        }

        if valid_mode & 0o777 == 0o777 {
            warn!(
                "Warning: 0777 permissions requested on path: {}",
                path.as_ref().display()
            );
        }

        let confined = self.resolve(path)?;
        let abs = confined.resolved_absolute();

        #[cfg(unix)]
        {
            let perm = fs::Permissions::from_mode(valid_mode);
            fs::set_permissions(abs, perm)
                .map_err(|e| FsError::io(confined.relative_posix(), e))?;
        }

        #[cfg(not(unix))]
        {
            let _ = abs;
        }

        Ok(())
    }

    /// Changes ownership of a file or directory with tenant security boundaries.
    ///
    /// Per PRD Section 5.3: Tenants cannot change file ownership (`chown dibatasi`).
    /// Non-admin callers attempting to set UID/GID different from their own will receive
    /// `FsError::PermissionDenied`.
    /// An admin flag never bypasses the OS permissions of the worker process.
    pub fn chown(
        &self,
        path: impl AsRef<Path>,
        uid: Option<u32>,
        gid: Option<u32>,
        caller_uid: u32,
        is_admin: bool,
    ) -> Result<(), FsError> {
        if !is_admin && uid.is_some_and(|target_uid| target_uid != caller_uid) {
            return Err(FsError::PermissionDenied {
                path: path.as_ref().to_string_lossy().to_string(),
                reason: "Penyewa tidak diizinkan mengubah kepemilikan berkas ke pengguna lain"
                    .to_string(),
            });
        }

        let confined = self.resolve(path)?;
        let abs = confined.resolved_absolute();

        #[cfg(unix)]
        {
            std::os::unix::fs::chown(abs, uid, gid)
                .map_err(|error| FsError::io(confined.relative_posix(), error))
        }

        #[cfg(not(unix))]
        {
            let _ = (abs, uid, gid);
            Err(FsError::io(
                confined.relative_posix(),
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Changing file ownership requires Unix",
                ),
            ))
        }
    }

    /// Recursively sets file and directory permissions with distinct modes.
    pub fn chmod_recursive(
        &self,
        path: impl AsRef<Path>,
        file_mode: u32,
        dir_mode: u32,
    ) -> Result<usize, FsError> {
        let confined = self.resolve(path)?;
        let abs = confined.resolved_absolute();

        let mut count = 0;
        self.chmod_recursive_internal(abs, file_mode & 0o7777, dir_mode & 0o7777, &mut count)?;
        Ok(count)
    }

    fn chmod_recursive_internal(
        &self,
        target: &Path,
        file_mode: u32,
        dir_mode: u32,
        count: &mut usize,
    ) -> Result<(), FsError> {
        let meta = fs::symlink_metadata(target)
            .map_err(|e| FsError::io(target.to_string_lossy().to_string(), e))?;

        if meta.file_type().is_symlink() {
            return Ok(());
        }

        #[cfg(unix)]
        {
            let mode = if meta.is_dir() { dir_mode } else { file_mode };
            let perm = fs::Permissions::from_mode(mode);
            fs::set_permissions(target, perm)
                .map_err(|e| FsError::io(target.to_string_lossy().to_string(), e))?;
        }

        #[cfg(not(unix))]
        {
            let _ = (file_mode, dir_mode);
        }

        *count += 1;

        if meta.is_dir() {
            let read_dir = fs::read_dir(target)
                .map_err(|e| FsError::io(target.to_string_lossy().to_string(), e))?;
            for entry in read_dir {
                let entry =
                    entry.map_err(|e| FsError::io(target.to_string_lossy().to_string(), e))?;
                self.chmod_recursive_internal(&entry.path(), file_mode, dir_mode, count)?;
            }
        }

        Ok(())
    }

    /// Writes a chunk of bytes to a file at a specific offset.
    /// Creates the file if it does not exist when offset == 0.
    pub fn write_chunk(
        &self,
        path: impl AsRef<Path>,
        offset: u64,
        data: &[u8],
    ) -> Result<u64, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|e| FsError::io(confined.relative_posix(), e))?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(offset == 0 && !abs_path.exists())
            .read(true)
            .write(true)
            .open(abs_path)
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        file.seek(SeekFrom::Start(offset))
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        file.write_all(data)
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        file.flush()
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        let new_offset = file
            .stream_position()
            .map_err(|e| FsError::io(confined.relative_posix(), e))?;

        Ok(new_offset)
    }

    /// Creates a zip archive of a directory and returns the bytes.
    pub fn zip_directory(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, FsError> {
        let confined = self.resolve(path)?;
        let abs_path = confined.resolved_absolute();

        if !abs_path.is_dir() {
            return Err(FsError::NotADirectory {
                path: confined.relative_posix(),
            });
        }

        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);

            self.add_dir_to_zip(
                &mut zip,
                abs_path,
                &PathBuf::new(),
                &options,
                confined.relative_posix(),
            )?;
            zip.finish().map_err(|e| FsError::Archive(e.to_string()))?;
        }

        Ok(buffer.into_inner())
    }

    fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(
        &self,
        zip: &mut zip::ZipWriter<W>,
        current_dir: &Path,
        rel_prefix: &Path,
        options: &zip::write::SimpleFileOptions,
        err_path: String,
    ) -> Result<(), FsError> {
        let entries = fs::read_dir(current_dir).map_err(|e| FsError::io(&err_path, e))?;

        for entry in entries {
            let entry = entry.map_err(|e| FsError::io(&err_path, e))?;
            let path = entry.path();
            let name = entry.file_name();
            let entry_rel = rel_prefix.join(name);
            let entry_rel_str = entry_rel.to_string_lossy().replace('\\', "/");

            if path.is_dir() {
                zip.add_directory(format!("{}/", entry_rel_str), *options)
                    .map_err(|e| FsError::Archive(e.to_string()))?;
                self.add_dir_to_zip(zip, &path, &entry_rel, options, err_path.clone())?;
            } else if path.is_file() {
                zip.start_file(&entry_rel_str, *options)
                    .map_err(|e| FsError::Archive(e.to_string()))?;
                let content = fs::read(&path).map_err(|e| FsError::io(&err_path, e))?;
                zip.write_all(&content)
                    .map_err(|e| FsError::io(&err_path, e))?;
            }
        }

        Ok(())
    }
}
