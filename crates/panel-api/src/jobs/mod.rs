//! Asynchronous job execution system, worker loop, and real-time SSE progress events.

pub mod events;
pub mod handlers;
pub mod registry;
pub mod runner;
pub mod worker;

pub use events::JobProgressEvent;
pub use handlers::*;
pub use registry::{
    EchoHandler, FailHandler, JobContext, JobFuture, JobHandler, JobRegistry, SleepHandler,
};
pub use runner::JobRunner;
pub use worker::{process_one_job, run_worker};
