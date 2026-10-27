use crate::attribution::SiteAttributor;
use crate::error::AnalyticsError;
use crate::models::{LogBatch, TailerEngineConfig, TailerStats};
use crate::storage::AnalyticsStorage;
use crate::tailer::LogFileTailer;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use tokio_util::sync::CancellationToken;

/// Orchestrator for watching, tailing, attributing, and checkpointing multiple site log files.
pub struct LogTailerEngine {
    config: TailerEngineConfig,
    watch_dirs: Arc<RwLock<Vec<PathBuf>>>,
    attributor: Arc<SiteAttributor>,
    storage: Option<AnalyticsStorage>,
    tailers: Arc<RwLock<HashMap<PathBuf, LogFileTailer>>>,
    batch_tx: mpsc::Sender<LogBatch>,
    notify_signal: Arc<Notify>,
    lines_counter: Arc<AtomicU64>,
    bytes_counter: Arc<AtomicU64>,
    batches_counter: Arc<AtomicU64>,
}

impl LogTailerEngine {
    /// Create a new `LogTailerEngine` instance along with the consumer receiver.
    pub fn new(
        config: TailerEngineConfig,
        storage: Option<AnalyticsStorage>,
        attributor: Option<Arc<SiteAttributor>>,
    ) -> (Self, mpsc::Receiver<LogBatch>) {
        let (batch_tx, batch_rx) = mpsc::channel(config.channel_capacity);
        let attributor = attributor.unwrap_or_else(|| Arc::new(SiteAttributor::new()));
        let tailers = Arc::new(RwLock::new(HashMap::new()));
        let notify_signal = Arc::new(Notify::new());
        let watch_dirs = Arc::new(RwLock::new(config.watch_directories.clone()));

        let engine = Self {
            config,
            watch_dirs,
            attributor,
            storage,
            tailers,
            batch_tx,
            notify_signal,
            lines_counter: Arc::new(AtomicU64::new(0)),
            bytes_counter: Arc::new(AtomicU64::new(0)),
            batches_counter: Arc::new(AtomicU64::new(0)),
        };

        (engine, batch_rx)
    }

    /// Access the site attributor to register custom mappings.
    pub fn attributor(&self) -> &Arc<SiteAttributor> {
        &self.attributor
    }

    /// Return a snapshot of tailer engine statistics.
    pub async fn stats(&self) -> TailerStats {
        let active = self.tailers.read().await.len();
        TailerStats {
            active_tailers: active,
            total_lines_processed: self.lines_counter.load(Ordering::Relaxed),
            total_bytes_processed: self.bytes_counter.load(Ordering::Relaxed),
            total_batches_processed: self.batches_counter.load(Ordering::Relaxed),
        }
    }

    /// Check if a log file path is currently registered and tracked.
    pub async fn is_tracking_path(&self, path: &Path) -> bool {
        self.tailers.read().await.contains_key(path)
    }

    /// List all currently tracked log file paths.
    pub async fn tracked_paths(&self) -> Vec<PathBuf> {
        self.tailers.read().await.keys().cloned().collect()
    }

    /// Number of active log file tailers.
    pub async fn active_tailers_count(&self) -> usize {
        self.tailers.read().await.len()
    }

    /// Explicitly register a site log file to track.
    pub async fn register_log_file(
        &self,
        site_id: i64,
        path: impl Into<PathBuf>,
    ) -> Result<(), AnalyticsError> {
        let path = path.into();
        let mut tailer = LogFileTailer::new(&path, site_id, self.config.batch_size);

        // Restore checkpoint from storage if available
        if let Some(storage) = &self.storage {
            let path_str = path.to_string_lossy().to_string();
            if let Some(saved) = storage.get_log_checkpoint(&path_str).await? {
                tailer.apply_checkpoint(&saved);
            }
        }

        self.attributor.register_path(&path, site_id);
        let mut map = self.tailers.write().await;
        map.insert(path, tailer);
        self.notify_signal.notify_one();
        Ok(())
    }

    /// Unregister a log file when a site is deleted or updated.
    pub async fn unregister_log_file(&self, path: &Path) -> Result<(), AnalyticsError> {
        let mut map = self.tailers.write().await;
        map.remove(path);

        if let Some(storage) = &self.storage {
            let path_str = path.to_string_lossy().to_string();
            storage.delete_log_checkpoint(&path_str).await?;
        }
        Ok(())
    }

    /// Add a directory to watch for site log files dynamically.
    pub async fn add_watch_directory(&self, dir: impl Into<PathBuf>) {
        let mut dirs = self.watch_dirs.write().await;
        let d = dir.into();
        if !dirs.contains(&d) {
            dirs.push(d);
            self.notify_signal.notify_one();
        }
    }

    /// Discover and register all matching log files in the configured watch directories.
    pub async fn discover_logs(&self) -> Result<usize, AnalyticsError> {
        let mut discovered = 0;
        let dirs = self.watch_dirs.read().await.clone();
        for dir in &dirs {
            if !dir.exists() {
                continue;
            }

            discovered += self.scan_directory_recursive(dir).await?;
        }
        Ok(discovered)
    }

    async fn scan_directory_recursive(&self, dir: &Path) -> Result<usize, AnalyticsError> {
        let mut count = 0;
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(e) => e,
            Err(_) => return Ok(0),
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                count += Box::pin(self.scan_directory_recursive(&path)).await?;
            } else if path.is_file() {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let is_log = (file_name.ends_with(".log") || file_name.contains("access"))
                    && !file_name.contains(".log.")
                    && !file_name.ends_with(".gz")
                    && !file_name.ends_with(".xz")
                    && !file_name.ends_with(".1")
                    && !file_name.ends_with(".old");
                let site_id_opt = if is_log {
                    self.attributor.resolve_site_id(&path, None)
                } else {
                    None
                };
                if let Some(site_id) = site_id_opt {
                    let mut map = self.tailers.write().await;
                    if let std::collections::hash_map::Entry::Vacant(e) = map.entry(path.clone()) {
                        let mut tailer = LogFileTailer::new(&path, site_id, self.config.batch_size);
                        if let Some(storage) = &self.storage {
                            let path_str = path.to_string_lossy().to_string();
                            if let Ok(Some(saved)) = storage.get_log_checkpoint(&path_str).await {
                                tailer.apply_checkpoint(&saved);
                            }
                        }
                        e.insert(tailer);
                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }

    /// Execute a single tailing iteration across all registered log files.
    /// Returns the number of batches read and dispatched.
    pub async fn process_all_once(&self) -> Result<usize, AnalyticsError> {
        let mut map = self.tailers.write().await;
        let mut batch_count = 0;

        for (path, tailer) in map.iter_mut() {
            while let Some(batch) = tailer.read_next_batch().await? {
                let lines_in_batch = batch.entries.len();
                let bytes_in_batch = batch.entries.iter().map(|e| e.body_bytes_sent).sum::<u64>();

                // Persist updated checkpoint
                if let Some(storage) = &self.storage {
                    let checkpoint = tailer.to_checkpoint();
                    storage.save_log_checkpoint(&checkpoint).await?;
                }

                // Push to bounded MPSC channel (applying backpressure if full)
                if self.batch_tx.send(batch).await.is_err() {
                    tracing::warn!("Log consumer channel closed");
                    return Ok(batch_count);
                }

                self.lines_counter
                    .fetch_add(lines_in_batch as u64, Ordering::Relaxed);
                self.bytes_counter
                    .fetch_add(bytes_in_batch, Ordering::Relaxed);
                self.batches_counter.fetch_add(1, Ordering::Relaxed);
                batch_count += 1;
            }
            let _ = path;
        }

        Ok(batch_count)
    }

    /// Start file system watcher in background for instant modification triggers.
    async fn setup_filesystem_watcher(&self) -> Option<RecommendedWatcher> {
        let notify_signal = Arc::clone(&self.notify_signal);
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let should_notify = match res {
                Ok(event) => event.kind.is_modify() || event.kind.is_create(),
                Err(_) => false,
            };
            if should_notify {
                notify_signal.notify_one();
            }
        })
        .ok()?;

        let dirs = self.watch_dirs.read().await.clone();
        for dir in &dirs {
            if dir.exists() {
                let _ = watcher.watch(dir, RecursiveMode::Recursive);
            }
        }

        Some(watcher)
    }

    async fn handle_event_trigger(&self) {
        let _ = self.discover_logs().await;
        if let Err(e) = self.process_all_once().await {
            tracing::error!(error = %e, "Error during event-driven tailing iteration");
        }
    }

    async fn handle_tick_trigger(&self) {
        let _ = self.discover_logs().await;
        if let Err(e) = self.process_all_once().await {
            tracing::error!(error = %e, "Error during periodic tailing iteration");
        }
    }

    /// Run the tailer engine background loop until cancellation is signaled.
    #[allow(clippy::cognitive_complexity)]
    pub async fn run_loop(
        self: Arc<Self>,
        cancel: CancellationToken,
    ) -> Result<(), AnalyticsError> {
        let _watcher = self.setup_filesystem_watcher().await;
        let poll_interval = self.config.poll_interval;
        let mut ticker = tokio::time::interval(poll_interval);

        tracing::info!(
            interval = ?poll_interval,
            "Log tailer engine background loop started"
        );

        while !cancel.is_cancelled() {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("Log tailer engine cancellation received");
                    break;
                }
                _ = self.notify_signal.notified() => {
                    self.handle_event_trigger().await;
                }
                _ = ticker.tick() => {
                    self.handle_tick_trigger().await;
                }
            }
        }

        tracing::info!("Log tailer engine background loop stopped cleanly");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, tempdir};

    #[tokio::test]
    async fn test_engine_register_and_process() {
        let mut temp = NamedTempFile::new().expect("temp file");
        let path = temp.path().to_path_buf();

        let line = r#"{"time_local":"2026-10-10T15:00:00Z","remote_addr":"192.168.1.10","request_method":"GET","request_uri":"/index.html","status":200,"body_bytes_sent":1024,"request_time":0.005,"host":"mysite.org"}"#;
        writeln!(temp, "{line}").expect("write line");
        temp.flush().expect("flush");

        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("in-memory db");

        let config = TailerEngineConfig {
            watch_directories: vec![],
            poll_interval: std::time::Duration::from_millis(50),
            batch_size: 50,
            channel_capacity: 10,
        };

        let (engine, mut rx) = LogTailerEngine::new(config, Some(storage.clone()), None);
        engine
            .register_log_file(77, &path)
            .await
            .expect("register file");

        let processed = engine.process_all_once().await.expect("process once");
        assert_eq!(processed, 1);

        let batch = rx.recv().await.expect("receive batch");
        assert_eq!(batch.site_id, 77);
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.entries[0].uri, "/index.html");

        // Verify checkpoint was persisted
        let path_str = path.to_string_lossy().to_string();
        let cp = storage
            .get_log_checkpoint(&path_str)
            .await
            .expect("get checkpoint")
            .expect("checkpoint exists");
        assert_eq!(cp.site_id, 77);
        assert!(cp.file_offset > 0);
    }

    #[tokio::test]
    async fn test_engine_discovery() {
        let dir = tempdir().expect("temp dir");
        let site_dir = dir.path().join("site-45");
        tokio::fs::create_dir_all(&site_dir).await.expect("mkdir");
        let log_file = site_dir.join("access.log");
        tokio::fs::write(&log_file, "{\"time_local\":\"2026-10-10T12:00:00Z\",\"remote_addr\":\"1.1.1.1\",\"request_method\":\"GET\",\"request_uri\":\"/\",\"status\":200}\n").await.expect("write");

        let storage = AnalyticsStorage::connect_in_memory()
            .await
            .expect("in-memory db");

        let config = TailerEngineConfig {
            watch_directories: vec![dir.path().to_path_buf()],
            poll_interval: std::time::Duration::from_millis(50),
            batch_size: 10,
            channel_capacity: 10,
        };

        let (engine, mut rx) = LogTailerEngine::new(config, Some(storage), None);
        let discovered = engine.discover_logs().await.expect("discover logs");
        assert_eq!(discovered, 1);

        let processed = engine.process_all_once().await.expect("process once");
        assert_eq!(processed, 1);

        let batch = rx.recv().await.expect("recv");
        assert_eq!(batch.site_id, 45);
        assert_eq!(batch.entries[0].client_ip, "1.1.1.1");
    }
}
