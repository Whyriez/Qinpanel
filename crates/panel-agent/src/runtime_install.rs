//! Cancellation on disconnect for typed binary installers; never used for APT.
use crate::AgentError;
use panel_proto::RpcStream;
use tokio_util::sync::CancellationToken;

pub(crate) async fn connected<T, F, R>(
    rpc: &mut RpcStream<T>,
    lock: &'static tokio::sync::Mutex<()>,
    operation: F,
) -> Result<R, AgentError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnOnce(CancellationToken) -> Result<R, AgentError> + Send + 'static,
    R: Send + 'static,
{
    let cancel = CancellationToken::new();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let operation_cancel = cancel.clone();
    let work = async move {
        let guard = tokio::select! {
            biased;
            _ = operation_cancel.cancelled() => return Err(AgentError::Internal("Pemasangan runtime dibatalkan sebelum dimulai".into())),
            guard = lock.lock() => guard,
        };
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            operation(operation_cancel)
        })
        .await
        .map_err(|error| {
            AgentError::Internal(format!("Worker pemasangan runtime gagal: {error}"))
        })?
    };
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => result,
        _ = rpc.recv_request() => {
            cancel.cancel();
            // Keep the mutation lock until the child is reaped and cleanup completes.
            let _ = work.await;
            Err(AgentError::Internal("Pemasangan runtime dibatalkan karena koneksi pemanggil ditutup".into()))
        }
    }
}
