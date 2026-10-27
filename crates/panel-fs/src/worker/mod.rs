//! `fs-worker` process model, communication protocol, and worker pool.
//!
//! Complies with PRD Section 5.3 and Task P5-T2:
//! - Dropped-privilege worker process model
//! - IPC protocol for filesystem operations
//! - Per-tenant worker pooling with idle timeout and automatic recovery

pub mod handler;
pub mod pool;
pub mod process;
pub mod protocol;

pub use handler::FsWorkerHandler;
pub use pool::{FsWorkerPool, FsWorkerPoolConfig, TenantWorkerKey};
pub use protocol::{FsWorkerRequest, FsWorkerResponse};
