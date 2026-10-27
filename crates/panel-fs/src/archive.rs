//! Safe archive compression and extraction module.
//!
//! Complies with PRD Section 5.3, Section 6, and Task P5-T5:
//! - Anti Zip-Slip: Prevents extraction of entries escaping extraction directory
//! - Anti Symlink Escaping: Rejects or sanitizes symlinks escaping root jail
//! - Anti Zip-Bomb: Guardrails against compression ratio (>100x), excessive entry counts (>50k), and excessive bytes (>10GB)
//! - Supports ZIP and TAR.GZ formats

use crate::error::FsError;
use crate::fs::ConfinedFs;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Supported archive formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    /// Infers archive format from filename or path extension.
    pub fn infer_from_path(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else {
            None
        }
    }
}

/// Guardrail thresholds to prevent zip-bomb, excessive entry exhaustion, and disk overflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractLimits {
    /// Maximum allowed ratio between uncompressed data and compressed archive size.
    pub max_ratio: u64,
    /// Maximum number of file/folder entries permitted in a single archive.
    pub max_entries: usize,
    /// Maximum total uncompressed byte size allowed across all extracted entries.
    pub max_uncompressed_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_ratio: 100,
            max_entries: 50_000,
            max_uncompressed_bytes: 10 * 1024 * 1024 * 1024, // 10 GB
        }
    }
}

/// Statistics and metadata returned upon completing archive operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveResult {
    pub entries_processed: usize,
    pub total_bytes: u64,
    pub target_path: String,
}

/// Sanitizes relative archive entry paths against Zip-Slip traversal.
pub fn sanitize_archive_entry_path(entry_path: &str, dest_dir: &str) -> Result<PathBuf, FsError> {
    // 1. Strip any leading slashes or volume prefixes
    let trimmed = entry_path.trim_start_matches('/').trim_start_matches('\\');

    // 2. Reject path traversal elements and null bytes
    if trimmed.contains('\0') {
        return Err(FsError::InvalidPath(
            "Archive entry path contains null byte".into(),
        ));
    }

    let p = Path::new(trimmed);
    for component in p.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err(FsError::PathTraversalAttempt {
                    path: entry_path.to_string(),
                    detail: "Archive entry contains parent directory (..) navigation".into(),
                });
            }
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                return Err(FsError::PathTraversalAttempt {
                    path: entry_path.to_string(),
                    detail: "Archive entry contains absolute root or volume prefix".into(),
                });
            }
            std::path::Component::Normal(part) => {
                let s = part.to_string_lossy();
                if s.contains('/') || s.contains('\\') || s.contains("..") {
                    return Err(FsError::PathTraversalAttempt {
                        path: entry_path.to_string(),
                        detail: "Archive entry contains invalid nested separator".into(),
                    });
                }
            }
            std::path::Component::CurDir => {}
        }
    }

    // 3. Build destination relative path
    let dest_buf = PathBuf::from(dest_dir.trim_matches('/'));
    let combined = if dest_buf.as_os_str().is_empty() {
        PathBuf::from(trimmed)
    } else {
        dest_buf.join(trimmed)
    };

    Ok(combined)
}

/// Compresses one or more paths into a ZIP archive within the confined filesystem.
pub fn compress_zip(
    fs: &ConfinedFs,
    source_paths: &[String],
    destination: &str,
) -> Result<ArchiveResult, FsError> {
    if source_paths.is_empty() {
        return Err(FsError::InvalidPath(
            "No source paths specified for compression".into(),
        ));
    }

    let dest_resolved = fs
        .resolve(Path::new(destination))?
        .resolved_absolute()
        .to_path_buf();
    if let Some(parent) = dest_resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|e| FsError::Io {
            path: destination.into(),
            detail: e.to_string(),
        })?;
    }

    let dest_file = File::create(&dest_resolved).map_err(|e| FsError::Io {
        path: destination.into(),
        detail: format!("Failed to create destination archive: {e}"),
    })?;

    let mut zip_writer = zip::ZipWriter::new(dest_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut entries_count = 0usize;
    let mut total_bytes = 0u64;

    for src in source_paths {
        let src_resolved = fs
            .resolve(Path::new(src))?
            .resolved_absolute()
            .to_path_buf();
        if !src_resolved.exists() {
            return Err(FsError::NotFound { path: src.clone() });
        }

        let base_name = Path::new(src)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(src);

        if src_resolved.is_dir() {
            add_dir_to_zip(
                &mut zip_writer,
                &src_resolved,
                base_name,
                options,
                &mut entries_count,
                &mut total_bytes,
            )?;
        } else {
            add_file_to_zip(
                &mut zip_writer,
                &src_resolved,
                base_name,
                options,
                &mut entries_count,
                &mut total_bytes,
            )?;
        }
    }

    zip_writer
        .finish()
        .map_err(|e| FsError::Archive(format!("Failed to finalize zip archive: {e}")))?;

    info!(
        "Successfully created zip archive {} with {} entries ({} bytes)",
        destination, entries_count, total_bytes
    );

    Ok(ArchiveResult {
        entries_processed: entries_count,
        total_bytes,
        target_path: destination.to_string(),
    })
}

fn add_file_to_zip<W: Write + std::io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    file_path: &Path,
    archive_name: &str,
    options: zip::write::SimpleFileOptions,
    count: &mut usize,
    bytes: &mut u64,
) -> Result<(), FsError> {
    writer
        .start_file(archive_name, options)
        .map_err(|e| FsError::Archive(format!("Failed to write zip entry {archive_name}: {e}")))?;

    let mut f = File::open(file_path).map_err(|e| FsError::Io {
        path: file_path.display().to_string(),
        detail: e.to_string(),
    })?;

    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer).map_err(|e| FsError::Io {
            path: file_path.display().to_string(),
            detail: e.to_string(),
        })?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buffer[..n])
            .map_err(|e| FsError::Archive(e.to_string()))?;
        *bytes = bytes.saturating_add(n as u64);
    }

    *count += 1;
    Ok(())
}

fn add_dir_to_zip<W: Write + std::io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    dir_path: &Path,
    prefix: &str,
    options: zip::write::SimpleFileOptions,
    count: &mut usize,
    bytes: &mut u64,
) -> Result<(), FsError> {
    let dir_name = if prefix.ends_with('/') {
        prefix.to_string()
    } else {
        format!("{prefix}/")
    };

    writer
        .add_directory(&dir_name, options)
        .map_err(|e| FsError::Archive(format!("Failed to add zip directory {dir_name}: {e}")))?;
    *count += 1;

    let read_dir = std::fs::read_dir(dir_path).map_err(|e| FsError::Io {
        path: dir_path.display().to_string(),
        detail: e.to_string(),
    })?;

    for entry_res in read_dir {
        let entry = entry_res.map_err(|e| FsError::Io {
            path: dir_path.display().to_string(),
            detail: e.to_string(),
        })?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let entry_prefix = format!("{prefix}/{name}");

        if path.is_dir() {
            add_dir_to_zip(writer, &path, &entry_prefix, options, count, bytes)?;
        } else if path.is_file() {
            add_file_to_zip(writer, &path, &entry_prefix, options, count, bytes)?;
        }
    }

    Ok(())
}

/// Compresses one or more paths into a TAR.GZ archive within the confined filesystem.
pub fn compress_tar_gz(
    fs: &ConfinedFs,
    source_paths: &[String],
    destination: &str,
) -> Result<ArchiveResult, FsError> {
    if source_paths.is_empty() {
        return Err(FsError::InvalidPath(
            "No source paths specified for compression".into(),
        ));
    }

    let dest_resolved = fs
        .resolve(Path::new(destination))?
        .resolved_absolute()
        .to_path_buf();
    if let Some(parent) = dest_resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|e| FsError::Io {
            path: destination.into(),
            detail: e.to_string(),
        })?;
    }

    let dest_file = File::create(&dest_resolved).map_err(|e| FsError::Io {
        path: destination.into(),
        detail: format!("Failed to create destination tar.gz archive: {e}"),
    })?;

    let enc = GzEncoder::new(dest_file, Compression::default());
    let mut tar_builder = tar::Builder::new(enc);

    let mut entries_count = 0usize;
    let mut total_bytes = 0u64;

    for src in source_paths {
        let src_resolved = fs
            .resolve(Path::new(src))?
            .resolved_absolute()
            .to_path_buf();
        if !src_resolved.exists() {
            return Err(FsError::NotFound { path: src.clone() });
        }

        let base_name = Path::new(src)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(src);

        if src_resolved.is_dir() {
            tar_builder
                .append_dir_all(base_name, &src_resolved)
                .map_err(|e| FsError::Archive(format!("Failed to append dir to tar: {e}")))?;
        } else {
            let mut file = File::open(&src_resolved).map_err(|e| FsError::Io {
                path: src_resolved.display().to_string(),
                detail: e.to_string(),
            })?;
            tar_builder
                .append_file(base_name, &mut file)
                .map_err(|e| FsError::Archive(format!("Failed to append file to tar: {e}")))?;
        }
        entries_count += 1;
    }

    let enc = tar_builder
        .into_inner()
        .map_err(|e| FsError::Archive(format!("Failed to finalize tar archive: {e}")))?;
    enc.finish()
        .map_err(|e| FsError::Archive(format!("Failed to finalize gzip stream: {e}")))?;

    if let Ok(meta) = std::fs::metadata(&dest_resolved) {
        total_bytes = meta.len();
    }

    info!(
        "Successfully created tar.gz archive {} with {} entries",
        destination, entries_count
    );

    Ok(ArchiveResult {
        entries_processed: entries_count,
        total_bytes,
        target_path: destination.to_string(),
    })
}

/// Safely extracts a ZIP or TAR.GZ archive with anti-zip-slip, anti-symlink, and anti-zip-bomb guardrails.
pub fn extract_archive(
    fs: &ConfinedFs,
    archive_path: &str,
    destination_dir: &str,
    format: Option<ArchiveFormat>,
    limits: Option<ExtractLimits>,
) -> Result<ArchiveResult, FsError> {
    let fmt = format
        .or_else(|| ArchiveFormat::infer_from_path(archive_path))
        .ok_or_else(|| {
            FsError::InvalidPath(format!(
                "Cannot infer archive format from {archive_path}. Specify format explicitly."
            ))
        })?;

    let limits = limits.unwrap_or_default();

    let archive_resolved = fs
        .resolve(Path::new(archive_path))?
        .resolved_absolute()
        .to_path_buf();
    if !archive_resolved.is_file() {
        return Err(FsError::NotFound {
            path: archive_path.to_string(),
        });
    }

    let archive_size = archive_resolved.metadata().map(|m| m.len()).unwrap_or(1);

    // Ensure destination directory is created in jail
    fs.create_dir_all(Path::new(destination_dir))?;

    match fmt {
        ArchiveFormat::Zip => extract_zip_safe(
            fs,
            &archive_resolved,
            destination_dir,
            archive_size,
            &limits,
        ),
        ArchiveFormat::TarGz => extract_tar_gz_safe(
            fs,
            &archive_resolved,
            destination_dir,
            archive_size,
            &limits,
        ),
    }
}

fn extract_zip_safe(
    fs: &ConfinedFs,
    archive_resolved: &Path,
    destination_dir: &str,
    archive_size: u64,
    limits: &ExtractLimits,
) -> Result<ArchiveResult, FsError> {
    let file = File::open(archive_resolved).map_err(|e| FsError::Io {
        path: archive_resolved.display().to_string(),
        detail: e.to_string(),
    })?;

    let mut zip_archive = zip::ZipArchive::new(file)
        .map_err(|e| FsError::Archive(format!("Invalid zip archive: {e}")))?;

    let mut entries_count = 0usize;
    let mut total_uncompressed = 0u64;

    for i in 0..zip_archive.len() {
        entries_count += 1;
        if entries_count > limits.max_entries {
            return Err(FsError::Archive(format!(
                "Archive exceeds maximum allowed entry count ({}) - possible zip-bomb",
                limits.max_entries
            )));
        }

        let mut entry = zip_archive
            .by_index(i)
            .map_err(|e| FsError::Archive(format!("Failed to read zip entry index {i}: {e}")))?;

        let entry_name = entry
            .name()
            .map_err(|e| FsError::Archive(format!("Invalid zip entry name: {e}")))?
            .into_owned();

        // 1. Anti Zip-Slip validation
        let safe_rel_path = sanitize_archive_entry_path(&entry_name, destination_dir)?;

        if entry.is_dir() {
            fs.create_dir_all(&safe_rel_path)?;
            continue;
        }

        // 2. Reject symlink entries if policy denies or if unsafe
        if entry.is_symlink() {
            warn!("Rejected symlink entry {} in archive", entry_name);
            return Err(FsError::SymlinkForbidden { path: entry_name });
        }

        // 3. Ensure parent directory exists
        if let Some(parent) = safe_rel_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs.create_dir_all(parent)?;
        }

        // 4. Stream extraction with anti zip-bomb counters
        let dest_resolved = fs
            .resolve(&safe_rel_path)?
            .resolved_absolute()
            .to_path_buf();
        let mut out_file = File::create(&dest_resolved).map_err(|e| FsError::Io {
            path: safe_rel_path.display().to_string(),
            detail: e.to_string(),
        })?;

        let mut buffer = [0u8; 65536];
        loop {
            let n = entry
                .read(&mut buffer)
                .map_err(|e| FsError::Archive(e.to_string()))?;
            if n == 0 {
                break;
            }
            out_file.write_all(&buffer[..n]).map_err(|e| FsError::Io {
                path: safe_rel_path.display().to_string(),
                detail: e.to_string(),
            })?;

            total_uncompressed = total_uncompressed.saturating_add(n as u64);

            if total_uncompressed > limits.max_uncompressed_bytes {
                return Err(FsError::FileTooLarge {
                    size: total_uncompressed,
                    max_allowed: limits.max_uncompressed_bytes,
                });
            }

            // Anti zip-bomb ratio verification (check when exceeding 10MB)
            if total_uncompressed > 10 * 1024 * 1024 && archive_size > 0 {
                let ratio = total_uncompressed / archive_size;
                if ratio > limits.max_ratio {
                    return Err(FsError::Archive(format!(
                        "Decompression ratio ({}x) exceeded maximum safe limit of {}x - possible zip-bomb",
                        ratio, limits.max_ratio
                    )));
                }
            }
        }
    }

    Ok(ArchiveResult {
        entries_processed: entries_count,
        total_bytes: total_uncompressed,
        target_path: destination_dir.to_string(),
    })
}

fn extract_tar_gz_safe(
    fs: &ConfinedFs,
    archive_resolved: &Path,
    destination_dir: &str,
    archive_size: u64,
    limits: &ExtractLimits,
) -> Result<ArchiveResult, FsError> {
    let file = File::open(archive_resolved).map_err(|e| FsError::Io {
        path: archive_resolved.display().to_string(),
        detail: e.to_string(),
    })?;

    let gz = GzDecoder::new(file);
    let mut tar_archive = tar::Archive::new(gz);

    let mut entries_count = 0usize;
    let mut total_uncompressed = 0u64;

    let entries = tar_archive
        .entries()
        .map_err(|e| FsError::Archive(format!("Failed to read tar entries: {e}")))?;

    for entry_res in entries {
        entries_count += 1;
        if entries_count > limits.max_entries {
            return Err(FsError::Archive(format!(
                "Archive exceeds maximum allowed entry count ({}) - possible zip-bomb",
                limits.max_entries
            )));
        }

        let mut entry =
            entry_res.map_err(|e| FsError::Archive(format!("Corrupt tar entry: {e}")))?;
        let entry_path_buf = entry
            .path()
            .map_err(|e| FsError::Archive(format!("Invalid entry path in tar: {e}")))?
            .to_path_buf();
        let entry_name = entry_path_buf.to_string_lossy().into_owned();

        // 1. Anti Zip-Slip validation
        let safe_rel_path = sanitize_archive_entry_path(&entry_name, destination_dir)?;

        let header = entry.header();
        let entry_type = header.entry_type();

        if entry_type.is_dir() {
            fs.create_dir_all(&safe_rel_path)?;
            continue;
        }

        // 2. Reject unsafe symlinks/hardlinks
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            warn!("Rejected link entry {} in tar archive", entry_name);
            return Err(FsError::SymlinkForbidden { path: entry_name });
        }

        // 3. Ensure parent directory exists
        if let Some(parent) = safe_rel_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs.create_dir_all(parent)?;
        }

        // 4. Stream extraction with anti zip-bomb counters
        let dest_resolved = fs
            .resolve(&safe_rel_path)?
            .resolved_absolute()
            .to_path_buf();
        let mut out_file = File::create(&dest_resolved).map_err(|e| FsError::Io {
            path: safe_rel_path.display().to_string(),
            detail: e.to_string(),
        })?;

        let mut buffer = [0u8; 65536];
        loop {
            let n = entry
                .read(&mut buffer)
                .map_err(|e| FsError::Archive(e.to_string()))?;
            if n == 0 {
                break;
            }
            out_file.write_all(&buffer[..n]).map_err(|e| FsError::Io {
                path: safe_rel_path.display().to_string(),
                detail: e.to_string(),
            })?;

            total_uncompressed = total_uncompressed.saturating_add(n as u64);

            if total_uncompressed > limits.max_uncompressed_bytes {
                return Err(FsError::FileTooLarge {
                    size: total_uncompressed,
                    max_allowed: limits.max_uncompressed_bytes,
                });
            }

            if total_uncompressed > 10 * 1024 * 1024 && archive_size > 0 {
                let ratio = total_uncompressed / archive_size;
                if ratio > limits.max_ratio {
                    return Err(FsError::Archive(format!(
                        "Decompression ratio ({}x) exceeded maximum safe limit of {}x - possible zip-bomb",
                        ratio, limits.max_ratio
                    )));
                }
            }
        }
    }

    Ok(ArchiveResult {
        entries_processed: entries_count,
        total_bytes: total_uncompressed,
        target_path: destination_dir.to_string(),
    })
}
