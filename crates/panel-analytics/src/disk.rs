#![allow(unsafe_code)]

use crate::error::AnalyticsError;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct DiskStats {
    pub disk_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub inodes_total: u64,
    pub inodes_used: u64,
}

pub struct DiskUsageReader {
    mount_point: PathBuf,
    mock_stats: RwLock<Option<DiskStats>>,
}

impl DiskUsageReader {
    pub fn new(mount_point: impl AsRef<Path>) -> Self {
        Self {
            mount_point: mount_point.as_ref().to_path_buf(),
            mock_stats: RwLock::new(None),
        }
    }

    pub fn default_root() -> Self {
        Self::new("/")
    }

    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    /// Set mock stats for deterministic testing on any platform.
    pub fn set_mock_stats(&self, stats: DiskStats) {
        if let Ok(mut lock) = self.mock_stats.write() {
            *lock = Some(stats);
        }
    }

    /// Read disk and inode statistics for the configured mount point.
    pub fn read_disk_stats(&self) -> Result<DiskStats, AnalyticsError> {
        if let Ok(lock) = self.mock_stats.read()
            && let Some(ref mock) = *lock
        {
            return Ok(mock.clone());
        }

        #[cfg(unix)]
        {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;

            let c_path = CString::new(self.mount_point.as_os_str().as_bytes())
                .map_err(|e| AnalyticsError::System(format!("Invalid path for statvfs: {e}")))?;

            let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
            let res = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
            if res != 0 {
                return Err(AnalyticsError::Io(std::io::Error::last_os_error()));
            }

            let block_size = if stat.f_frsize > 0 {
                stat.f_frsize as u64
            } else {
                stat.f_bsize as u64
            };

            let disk_total_bytes = stat.f_blocks as u64 * block_size;
            let disk_free_bytes = stat.f_bavail as u64 * block_size;
            let disk_used_bytes = disk_total_bytes.saturating_sub(disk_free_bytes);

            let inodes_total = stat.f_files as u64;
            let inodes_free = stat.f_favail as u64;
            let inodes_used = inodes_total.saturating_sub(inodes_free);

            Ok(DiskStats {
                disk_total_bytes,
                disk_used_bytes,
                inodes_total,
                inodes_used,
            })
        }

        #[cfg(not(unix))]
        {
            // Fallback for non-unix development environments when no mock is set
            Ok(DiskStats {
                disk_total_bytes: 100 * 1024 * 1024 * 1024,
                disk_used_bytes: 25 * 1024 * 1024 * 1024,
                inodes_total: 10_000_000,
                inodes_used: 1_250_000,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disk_usage_reader_mock() {
        let reader = DiskUsageReader::default_root();
        reader.set_mock_stats(DiskStats {
            disk_total_bytes: 50 * 1024 * 1024 * 1024,
            disk_used_bytes: 10 * 1024 * 1024 * 1024,
            inodes_total: 5_000_000,
            inodes_used: 500_000,
        });

        let stats = reader.read_disk_stats().expect("read mock");
        assert_eq!(stats.disk_total_bytes, 50 * 1024 * 1024 * 1024);
        assert_eq!(stats.disk_used_bytes, 10 * 1024 * 1024 * 1024);
        assert_eq!(stats.inodes_total, 5_000_000);
        assert_eq!(stats.inodes_used, 500_000);
    }
}
