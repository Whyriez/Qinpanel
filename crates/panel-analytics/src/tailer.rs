use crate::error::AnalyticsError;
use crate::log_entry::NginxJsonLog;
use crate::models::{LogBatch, LogCheckpoint};
use chrono::Utc;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

#[cfg(unix)]
fn get_file_inode(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.ino()
}

#[cfg(windows)]
fn get_file_inode(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

#[cfg(not(any(unix, windows)))]
fn get_file_inode(_metadata: &std::fs::Metadata) -> u64 {
    1
}

/// Robust, rotation-resistant single log file tailer.
pub struct LogFileTailer {
    file_path: PathBuf,
    site_id: i64,
    current_inode: u64,
    current_offset: u64,
    batch_size: usize,
    last_processed_ts: i64,
}

impl LogFileTailer {
    /// Create a new tailer for a given log file.
    pub fn new(file_path: impl Into<PathBuf>, site_id: i64, batch_size: usize) -> Self {
        Self {
            file_path: file_path.into(),
            site_id,
            current_inode: 0,
            current_offset: 0,
            batch_size: batch_size.max(1),
            last_processed_ts: Utc::now().timestamp(),
        }
    }

    /// Restore tailer state from a previously saved checkpoint.
    pub fn apply_checkpoint(&mut self, checkpoint: &LogCheckpoint) {
        self.current_inode = checkpoint.inode;
        self.current_offset = checkpoint.file_offset;
        self.last_processed_ts = checkpoint.last_processed_ts;
    }

    /// Generate current checkpoint snapshot.
    pub fn to_checkpoint(&self) -> LogCheckpoint {
        LogCheckpoint {
            file_path: self.file_path.to_string_lossy().to_string(),
            site_id: self.site_id,
            inode: self.current_inode,
            file_offset: self.current_offset,
            last_processed_ts: self.last_processed_ts,
            updated_at: Utc::now().timestamp(),
        }
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn site_id(&self) -> i64 {
        self.site_id
    }

    pub fn current_offset(&self) -> u64 {
        self.current_offset
    }

    pub fn current_inode(&self) -> u64 {
        self.current_inode
    }

    /// Check if a rotated version of the file exists that matches the previous inode
    /// and still contains unread bytes from `self.current_offset`.
    fn check_rotated_candidate(&self) -> Option<PathBuf> {
        let parent = self.file_path.parent()?;
        let file_name = self.file_path.file_name()?.to_str()?;
        let candidates = [
            parent.join(format!("{file_name}.1")),
            parent.join(format!("{file_name}.old")),
            parent.join(format!("{file_name}.rotated")),
        ];

        candidates.into_iter().find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|meta| {
                let matches_inode = (self.current_inode == 0)
                    || (get_file_inode(&meta) == self.current_inode)
                    || cfg!(windows);
                matches_inode && meta.len() > self.current_offset
            })
        })
    }

    async fn try_drain_rotated_file(
        &mut self,
        file_inode: u64,
        rotated_path: &Path,
    ) -> Result<Option<LogBatch>, AnalyticsError> {
        let (entries, consumed_bytes) = self.read_batch_from_path(rotated_path).await?;
        if consumed_bytes > 0 {
            self.current_offset += consumed_bytes;
        }

        let rotated_len = std::fs::metadata(rotated_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if self.current_offset >= rotated_len {
            tracing::info!(
                path = %self.file_path.display(),
                rotated = %rotated_path.display(),
                "Finished draining rotated log file, switching to new file"
            );
            self.current_inode = file_inode;
            self.current_offset = 0;
        }

        if !entries.is_empty() {
            Ok(Some(LogBatch {
                file_path: self.file_path.to_string_lossy().to_string(),
                site_id: self.site_id,
                inode: self.current_inode,
                new_offset: self.current_offset,
                entries,
            }))
        } else {
            Ok(None)
        }
    }

    async fn handle_rotation_or_truncation(
        &mut self,
        file_inode: u64,
        file_len: u64,
    ) -> Result<Option<LogBatch>, AnalyticsError> {
        // 1. Detect rotation via rename/recreate (inode changed or rotated candidate present when file_len < offset)
        let is_inode_changed = self.current_inode != 0 && file_inode != self.current_inode;
        let is_offset_at_or_past_eof = file_len <= self.current_offset;

        let candidate = if is_inode_changed || is_offset_at_or_past_eof {
            self.check_rotated_candidate()
        } else {
            None
        };

        if let Some(rotated_path) = candidate {
            let maybe_batch = self
                .try_drain_rotated_file(file_inode, &rotated_path)
                .await?;
            if maybe_batch.is_some() {
                return Ok(maybe_batch);
            }
        }

        if is_inode_changed {
            tracing::info!(
                path = %self.file_path.display(),
                old_inode = self.current_inode,
                new_inode = file_inode,
                "Log file rotated (inode change detected), resetting offset to 0"
            );
            self.current_inode = file_inode;
            self.current_offset = 0;
        } else if file_len < self.current_offset {
            tracing::info!(
                path = %self.file_path.display(),
                file_len = file_len,
                current_offset = self.current_offset,
                "Log file truncated (copytruncate detected), resetting offset to 0"
            );
            self.current_offset = 0;
        } else if self.current_inode == 0 {
            self.current_inode = file_inode;
            if self.current_offset > file_len {
                self.current_offset = 0;
            }
        }

        Ok(None)
    }

    async fn read_batch_from_path(
        &mut self,
        path: &Path,
    ) -> Result<(Vec<crate::models::ParsedLogEntry>, u64), AnalyticsError> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(AnalyticsError::Io)?;
        file.seek(SeekFrom::Start(self.current_offset))
            .await
            .map_err(AnalyticsError::Io)?;

        let mut reader = BufReader::new(file);
        let mut entries = Vec::with_capacity(self.batch_size);
        let mut consumed_bytes = 0u64;

        while entries.len() < self.batch_size {
            let mut line = String::new();
            let bytes_read = reader
                .read_line(&mut line)
                .await
                .map_err(AnalyticsError::Io)?;

            if bytes_read == 0 {
                break;
            }

            if !line.ends_with('\n') {
                tracing::debug!(
                    path = %path.display(),
                    "Incomplete log line encountered at EOF, deferring to next poll"
                );
                break;
            }

            consumed_bytes += bytes_read as u64;

            match NginxJsonLog::parse_line(&line) {
                Ok(parsed) => {
                    let entry = parsed.into_entry(self.site_id);
                    self.last_processed_ts = entry.timestamp;
                    entries.push(entry);
                }
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        line = %line.trim(),
                        "Skipping corrupted log line"
                    );
                }
            }
        }

        Ok((entries, consumed_bytes))
    }

    /// Read the next batch of log entries from disk.
    ///
    /// Handles:
    /// 1. Rotation via file rename/recreate (`inode` changed), draining any unread bytes from `.1` / `.old` first.
    /// 2. Rotation via `copytruncate` (`file_len < offset`).
    /// 3. Incomplete lines at EOF (defers until line completes with newline).
    /// 4. Resumption from crash using saved offset.
    pub async fn read_next_batch(&mut self) -> Result<Option<LogBatch>, AnalyticsError> {
        let std_metadata = match std::fs::metadata(&self.file_path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(AnalyticsError::Io(e)),
        };

        let file_inode = get_file_inode(&std_metadata);
        let file_len = std_metadata.len();

        if let Some(batch) = self
            .handle_rotation_or_truncation(file_inode, file_len)
            .await?
        {
            return Ok(Some(batch));
        }

        if file_len <= self.current_offset {
            return Ok(None);
        }

        let file_path = self.file_path.clone();
        let (entries, consumed_bytes) = self.read_batch_from_path(&file_path).await?;
        if consumed_bytes > 0 {
            self.current_offset += consumed_bytes;
        }

        if entries.is_empty() {
            Ok(None)
        } else {
            Ok(Some(LogBatch {
                file_path: self.file_path.to_string_lossy().to_string(),
                site_id: self.site_id,
                inode: self.current_inode,
                new_offset: self.current_offset,
                entries,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_tailer_read_and_checkpoint() {
        let mut temp = NamedTempFile::new().expect("create temp file");
        let path = temp.path().to_path_buf();

        let line1 = r#"{"time_local":"2026-10-10T14:00:00Z","remote_addr":"10.0.0.1","request_method":"GET","request_uri":"/","status":200,"body_bytes_sent":500,"request_time":0.010,"host":"test.com"}"#;
        let line2 = r#"{"time_local":"2026-10-10T14:00:01Z","remote_addr":"10.0.0.2","request_method":"POST","request_uri":"/submit","status":201,"body_bytes_sent":200,"request_time":0.050,"host":"test.com"}"#;

        writeln!(temp, "{line1}").expect("write line1");
        writeln!(temp, "{line2}").expect("write line2");
        temp.flush().expect("flush");

        let mut tailer = LogFileTailer::new(&path, 100, 10);
        let batch = tailer
            .read_next_batch()
            .await
            .expect("read batch")
            .expect("has batch");

        assert_eq!(batch.site_id, 100);
        assert_eq!(batch.entries.len(), 2);
        assert_eq!(batch.entries[0].client_ip, "10.0.0.1");
        assert_eq!(batch.entries[1].client_ip, "10.0.0.2");
        assert!(tailer.current_offset() > 0);

        // Subsequent read has nothing
        let empty = tailer.read_next_batch().await.expect("read empty");
        assert!(empty.is_none());

        // Append line 3
        let line3 = r#"{"time_local":"2026-10-10T14:00:02Z","remote_addr":"10.0.0.3","request_method":"GET","request_uri":"/about","status":200,"body_bytes_sent":300,"request_time":0.015,"host":"test.com"}"#;
        writeln!(temp, "{line3}").expect("write line3");
        temp.flush().expect("flush");

        let batch2 = tailer
            .read_next_batch()
            .await
            .expect("read batch 2")
            .expect("has batch 2");
        assert_eq!(batch2.entries.len(), 1);
        assert_eq!(batch2.entries[0].client_ip, "10.0.0.3");
    }

    #[tokio::test]
    async fn test_tailer_handles_copytruncate() {
        let mut temp = NamedTempFile::new().expect("create temp file");
        let path = temp.path().to_path_buf();

        let line = r#"{"time_local":"2026-10-10T14:00:00Z","remote_addr":"10.0.0.1","request_method":"GET","request_uri":"/","status":200,"body_bytes_sent":500,"request_time":0.010,"host":"test.com"}"#;
        writeln!(temp, "{line}").expect("write line");
        writeln!(temp, "{line}").expect("write line 2");
        temp.flush().expect("flush");

        let mut tailer = LogFileTailer::new(&path, 55, 10);
        let _ = tailer.read_next_batch().await.expect("read batch");
        assert!(tailer.current_offset() > 0);

        // Truncate file
        let f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .expect("truncate");
        drop(f);

        // Write a new line in the truncated file
        let new_line = r#"{"time_local":"2026-10-10T14:05:00Z","remote_addr":"10.0.0.9","request_method":"GET","request_uri":"/new","status":200,"body_bytes_sent":100,"request_time":0.005,"host":"test.com"}"#;
        let mut f2 = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open");
        writeln!(f2, "{new_line}").expect("write new line");
        f2.flush().expect("flush");

        let batch = tailer
            .read_next_batch()
            .await
            .expect("read batch after truncation")
            .expect("batch present");
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.entries[0].uri, "/new");
    }

    #[tokio::test]
    async fn test_tailer_defers_incomplete_line() {
        let mut temp = NamedTempFile::new().expect("create temp file");
        let path = temp.path().to_path_buf();

        // Write incomplete line (no trailing newline)
        write!(temp, r#"{{"time_local":"2026-10-10T14:00:00Z""#).expect("write partial");
        temp.flush().expect("flush");

        let mut tailer = LogFileTailer::new(&path, 1, 10);
        let batch = tailer.read_next_batch().await.expect("read batch");
        assert!(batch.is_none());
        assert_eq!(tailer.current_offset(), 0);

        // Complete the line
        writeln!(
            temp,
            r#","remote_addr":"10.0.0.1","request_method":"GET","request_uri":"/","status":200,"body_bytes_sent":10,"request_time":0.01,"host":"test.com"}}"#
        )
        .expect("write rest");
        temp.flush().expect("flush");

        let batch2 = tailer
            .read_next_batch()
            .await
            .expect("read completed line")
            .expect("batch present");
        assert_eq!(batch2.entries.len(), 1);
        assert_eq!(batch2.entries[0].client_ip, "10.0.0.1");
    }
}
