#![allow(clippy::unwrap_used, clippy::expect_used)]
//! In-process worker and SQLite only: no AgentServer or system commands.
use panel_api::jobs::{
    EchoHandler, JobContext, JobFuture, JobHandler, JobRegistry, JobRunner, process_one_job,
};
use panel_db::{
    models::Job,
    pool::{init_pool, run_migrations},
};
use std::sync::Arc;

struct PendingNodeHandler;
impl JobHandler for PendingNodeHandler {
    fn run<'a>(&'a self, _job: &'a Job, ctx: &'a JobContext) -> JobFuture<'a> {
        Box::pin(async move {
            ctx.report_progress(15, Some("Pending Node RPC fixture"))
                .await
                .unwrap();
            ctx.run_cancellable_runtime_install(std::future::pending::<Option<String>>())
                .await
        })
    }
}

#[tokio::test]
async fn cancelled_node_rpc_releases_worker_for_queued_php_repository() {
    cancelled_runtime_releases_worker("node.install").await;
}

#[tokio::test]
async fn cancelled_python_rpc_releases_worker_for_queued_php_repository() {
    cancelled_runtime_releases_worker("python.install").await;
}

async fn cancelled_runtime_releases_worker(runtime: &str) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp
        .path()
        .join("jobs.db")
        .to_string_lossy()
        .replace('\\', "/");
    let pool = init_pool(&format!("sqlite://{path}")).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let mut registry = JobRegistry::new();
    registry.register(runtime, PendingNodeHandler);
    registry.register("php.repo.setup", EchoHandler);
    let runner = Arc::new(JobRunner::with_registry(pool, registry));
    let (node, _) = runner
        .enqueue(
            None,
            Some("01-node".into()),
            runtime.into(),
            "{}".into(),
            Some(1),
        )
        .await
        .unwrap();
    let (php, _) = runner
        .enqueue(
            None,
            Some("02-php".into()),
            "php.repo.setup".into(),
            "{}".into(),
            Some(1),
        )
        .await
        .unwrap();
    let mut events = runner.subscribe();
    let worker_runner = Arc::clone(&runner);
    let worker = tokio::spawn(async move {
        assert!(process_one_job(&worker_runner).await.unwrap());
        assert!(process_one_job(&worker_runner).await.unwrap());
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.job_id == node.id && event.progress == 15 {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        runner
            .job_repo
            .find_by_id(&php.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "queued"
    );
    runner.cancel(&node.id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), worker)
        .await
        .expect("cancelled Node RPC must not occupy the worker")
        .unwrap();
    assert_eq!(
        runner
            .job_repo
            .find_by_id(&node.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "cancelled"
    );
    assert_eq!(
        runner
            .job_repo
            .find_by_id(&php.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "succeeded"
    );
    let mut statuses = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.job_id == php.id {
            statuses.push(event.status);
        }
    }
    assert!(statuses.contains(&"running".to_owned()));
    assert!(statuses.contains(&"succeeded".to_owned()));
}
