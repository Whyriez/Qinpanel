//! `panel-db`: Database connection pooling, SQLite migrations, and repositories.

pub mod error;
pub mod models;
pub mod operations;
pub mod pool;
pub mod repositories;
pub mod snapshot;

pub use error::DbError;
pub use models::*;
pub use pool::{DbPool, init_pool, run_migrations};
pub use repositories::*;
