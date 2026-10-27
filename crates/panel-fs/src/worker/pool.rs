//! Worker session management, pooling, and idle timeout lifecycle.

#[cfg(debug_assertions)]
use super::handler::FsWorkerHandler;
use super::process::WorkerProcess;
use super::protocol::{FsWorkerRequest, FsWorkerResponse};
use crate::error::FsError;
#[cfg(debug_assertions)]
use crate::fs::ConfinedFs;
use crate::policy::SymlinkPolicy;
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(debug_assertions)]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

/// Tenant identification for routing file worker processes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantWorkerKey {
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

impl TenantWorkerKey {
    pub fn new(uid: u32, gid: u32, home: impl Into<PathBuf>) -> Self {
        Self {
            uid,
            gid,
            home: home.into(),
        }
    }
}

/// Configuration settings for the worker pool.
#[derive(Debug, Clone)]
pub struct FsWorkerPoolConfig {
    /// Inactivity duration before an idle worker process is terminated.
    pub idle_timeout: Duration,
    /// Maximum concurrent worker instances across all tenants.
    pub max_workers: usize,
    /// Symlink containment policy to enforce within the worker.
    pub symlink_policy: SymlinkPolicy,
}

impl Default for FsWorkerPoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(60),
            max_workers: 64,
            symlink_policy: SymlinkPolicy::DenyAll,
        }
    }
}

type WorkerSender = mpsc::Sender<(FsWorkerRequest, oneshot::Sender<FsWorkerResponse>)>;

struct ActiveWorker {
    tx: WorkerSender,
}

impl ActiveWorker {
    fn is_alive(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// Pool managing pooled `fs-worker` sessions per tenant with automatic idle reaping.
pub struct FsWorkerPool {
    config: FsWorkerPoolConfig,
    executable: Option<PathBuf>,
    workers: Mutex<HashMap<TenantWorkerKey, ActiveWorker>>,
}

impl FsWorkerPool {
    /// Creates a new worker pool with the specified configuration.
    pub fn new(config: FsWorkerPoolConfig) -> Self {
        // Pin the running image, including during an atomic package replacement.
        #[cfg(target_os = "linux")]
        let executable = PathBuf::from("/proc/self/exe");
        #[cfg(not(target_os = "linux"))]
        let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("panel-agent"));
        Self {
            config,
            executable: Some(executable),
            workers: Mutex::new(HashMap::new()),
        }
    }

    /// Explicit executable injection for native isolation tests; never sourced from HTTP input.
    pub fn with_executable(config: FsWorkerPoolConfig, executable: PathBuf) -> Self {
        Self {
            config,
            executable: Some(executable),
            workers: Mutex::new(HashMap::new()),
        }
    }

    /// Simulator for fixture tests only; absent from release builds.
    #[cfg(debug_assertions)]
    pub fn in_process_for_tests(config: FsWorkerPoolConfig) -> Self {
        Self {
            config,
            executable: None,
            workers: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the configured idle timeout duration.
    pub fn idle_timeout(&self) -> Duration {
        self.config.idle_timeout
    }

    /// Dispatches a filesystem request to a tenant's pooled worker.
    ///
    /// Spawns a new worker if none exists or if the previous worker terminated.
    pub async fn execute(
        &self,
        tenant: &TenantWorkerKey,
        request: FsWorkerRequest,
    ) -> Result<FsWorkerResponse, FsError> {
        let tx = self.get_or_spawn_worker(tenant)?;
        let (resp_tx, resp_rx) = oneshot::channel();
        if tx.send((request, resp_tx)).await.is_err() {
            self.evict_matching_worker(tenant, &tx);
            return Err(FsError::Io {
                path: "fs-worker".into(),
                detail: "Worker channel closed".into(),
            });
        }
        match resp_rx.await {
            Ok(response) => Ok(response),
            Err(_) => {
                self.evict_matching_worker(tenant, &tx);
                // Never replay a mutation whose reply was lost after it may have committed.
                Err(FsError::Io {
                    path: "fs-worker".into(),
                    detail: "Worker terminated; operation outcome may be unknown".into(),
                })
            }
        }
    }

    /// Returns an existing healthy worker or spawns a new one.
    fn get_or_spawn_worker(&self, tenant: &TenantWorkerKey) -> Result<WorkerSender, FsError> {
        let mut lock = self.workers.lock().map_err(|_| FsError::Io {
            path: tenant.home.display().to_string(),
            detail: "Worker pool mutex poisoned".into(),
        })?;

        if let Some(worker) = lock.get(tenant)
            && worker.is_alive()
        {
            return Ok(worker.tx.clone());
        }

        // Check max workers limit
        if lock.len() >= self.config.max_workers {
            // Force reap idle workers first
            let mut to_remove = Vec::new();
            for (k, w) in lock.iter() {
                if !w.is_alive() {
                    to_remove.push(k.clone());
                }
            }
            for k in to_remove {
                lock.remove(&k);
            }

            if lock.len() >= self.config.max_workers {
                return Err(FsError::Io {
                    path: tenant.home.display().to_string(),
                    detail: format!(
                        "Maximum worker pool capacity reached ({} workers)",
                        self.config.max_workers
                    ),
                });
            }
        }

        let tx = self.spawn_worker_instance(tenant)?;
        lock.insert(tenant.clone(), ActiveWorker { tx: tx.clone() });

        debug!(
            "Spawned new fs-worker for UID {} (jail: {})",
            tenant.uid,
            tenant.home.display()
        );

        Ok(tx)
    }

    fn spawn_worker_instance(&self, tenant: &TenantWorkerKey) -> Result<WorkerSender, FsError> {
        let (tx, mut rx) = mpsc::channel::<(FsWorkerRequest, oneshot::Sender<FsWorkerResponse>)>(8);
        let idle_timeout = self.config.idle_timeout;
        if let Some(executable) = &self.executable {
            let mut worker = WorkerProcess::spawn(executable, tenant, self.config.symlink_policy)?;
            tokio::spawn(async move {
                while let Ok(Some((request, response))) =
                    tokio::time::timeout(idle_timeout, rx.recv()).await
                {
                    let shutdown = matches!(request, FsWorkerRequest::Shutdown);
                    match worker.request(&request).await {
                        Ok(result) => {
                            let _ = response.send(result);
                        }
                        Err(error) => {
                            warn!(%error, "Isolated file worker failed; request will not be replayed");
                            worker.shutdown().await;
                            drop(response);
                            break;
                        }
                    }
                    if shutdown {
                        break;
                    }
                }
                worker.shutdown().await;
            });
        } else {
            #[cfg(debug_assertions)]
            {
                let handler = Arc::new(FsWorkerHandler::new(ConfinedFs::new(
                    &tenant.home,
                    self.config.symlink_policy,
                )?));
                tokio::spawn(async move {
                    while let Ok(Some((request, response))) =
                        tokio::time::timeout(idle_timeout, rx.recv()).await
                    {
                        let shutdown = matches!(request, FsWorkerRequest::Shutdown);
                        let _ = response.send(handler.handle(request));
                        if shutdown {
                            break;
                        }
                    }
                });
            }
            #[cfg(not(debug_assertions))]
            return Err(FsError::PermissionDenied {
                path: "fs-worker".into(),
                reason: "An isolated process is required".into(),
            });
        }
        Ok(tx)
    }

    fn evict_matching_worker(&self, tenant: &TenantWorkerKey, sender: &WorkerSender) {
        if let Ok(mut lock) = self.workers.lock()
            && lock
                .get(tenant)
                .is_some_and(|worker| worker.tx.same_channel(sender))
        {
            lock.remove(tenant);
        }
    }

    /// Evicts a terminated or broken worker from the pool.
    pub fn evict_worker(&self, tenant: &TenantWorkerKey) {
        if let Ok(mut lock) = self.workers.lock() {
            lock.remove(tenant);
        }
    }

    /// Evicts closed workers. Each worker enforces its own receive-side idle timeout;
    /// a busy worker is never reaped based on the age of its previous response.
    ///
    /// Returns the number of idle workers shut down.
    pub fn cleanup_idle(&self) -> usize {
        let mut to_reap = Vec::new();

        if let Ok(mut lock) = self.workers.lock() {
            for (key, worker) in lock.iter() {
                if !worker.is_alive() {
                    to_reap.push((key.clone(), worker.tx.clone()));
                }
            }

            for (key, tx) in to_reap.iter() {
                lock.remove(key);
                let (resp_tx, _) = oneshot::channel();
                let _ = tx.try_send((FsWorkerRequest::Shutdown, resp_tx));
            }
        }

        let reaped_count = to_reap.len();
        if reaped_count > 0 {
            info!("Reaped {reaped_count} idle fs-workers from pool");
        }
        reaped_count
    }

    /// Returns the number of currently active workers in the pool.
    pub fn active_worker_count(&self) -> usize {
        self.workers.lock().map(|l| l.len()).unwrap_or(0)
    }

    /// Gracefully shuts down all active workers in the pool.
    pub async fn shutdown_all(&self) {
        let mut workers_to_close = Vec::new();
        if let Ok(mut lock) = self.workers.lock() {
            for (_, worker) in lock.drain() {
                workers_to_close.push(worker.tx);
            }
        }

        for tx in workers_to_close {
            let (resp_tx, _) = oneshot::channel();
            let _ = tx.send((FsWorkerRequest::Shutdown, resp_tx)).await;
        }
    }
}
