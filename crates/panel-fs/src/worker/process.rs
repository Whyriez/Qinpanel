//! Bounded pipe transport to a separate, jailed panel-agent worker process.
use super::protocol::{FsWorkerRequest, FsWorkerResponse};
use crate::{FsError, SymlinkPolicy, TenantWorkerKey};
use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub const MAX_WORKER_FRAME: usize = 64 * 1024 * 1024;

pub struct WorkerProcess {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
}

impl WorkerProcess {
    pub fn spawn(
        executable: &Path,
        tenant: &TenantWorkerKey,
        policy: SymlinkPolicy,
    ) -> Result<Self, FsError> {
        let mut child = Command::new(executable)
            .arg("--fs-worker")
            .arg(tenant.uid.to_string())
            .arg(tenant.gid.to_string())
            .arg(&tenant.home)
            .arg(serde_json::to_string(&policy).map_err(transport_error)?)
            .env_clear()
            .env("LANG", "C.UTF-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(transport_error)?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| transport_error("Worker stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| transport_error("Worker stdout unavailable"))?;
        Ok(Self {
            child,
            input,
            output,
        })
    }

    pub async fn request(
        &mut self,
        request: &FsWorkerRequest,
    ) -> Result<FsWorkerResponse, FsError> {
        let bytes = serde_json::to_vec(request).map_err(transport_error)?;
        if bytes.len() > MAX_WORKER_FRAME {
            return Err(transport_error("Worker request exceeds frame limit"));
        }
        let exchange = async {
            self.input
                .write_u32(bytes.len() as u32)
                .await
                .map_err(transport_error)?;
            self.input
                .write_all(&bytes)
                .await
                .map_err(transport_error)?;
            self.input.flush().await.map_err(transport_error)?;
            let length = self.output.read_u32().await.map_err(transport_error)? as usize;
            if length > MAX_WORKER_FRAME {
                return Err(transport_error("Worker response exceeds frame limit"));
            }
            let mut response = vec![0; length];
            self.output
                .read_exact(&mut response)
                .await
                .map_err(transport_error)?;
            serde_json::from_slice(&response).map_err(transport_error)
        };
        tokio::time::timeout(Duration::from_secs(120), exchange)
            .await
            .map_err(|_| transport_error("Worker response timed out; outcome may be unknown"))?
    }

    pub async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

fn transport_error(error: impl std::fmt::Display) -> FsError {
    FsError::Io {
        path: "fs-worker".into(),
        detail: error.to_string(),
    }
}

pub fn read_request(reader: &mut impl Read) -> std::io::Result<Option<FsWorkerRequest>> {
    let mut header = [0; 4];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_WORKER_FRAME {
        return Err(std::io::Error::other("Worker frame too large"));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(std::io::Error::other)
}

pub fn write_response(writer: &mut impl Write, response: &FsWorkerResponse) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(response).map_err(std::io::Error::other)?;
    if bytes.len() > MAX_WORKER_FRAME {
        return Err(std::io::Error::other("Worker response too large"));
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversized_and_malformed_frames_before_dispatch() {
        assert!(
            read_request(&mut ((MAX_WORKER_FRAME + 1) as u32).to_be_bytes().as_slice()).is_err()
        );
        assert!(read_request(&mut [0, 0, 0, 1, b'{'].as_slice()).is_err());
        assert!(read_request(&mut [].as_slice()).unwrap().is_none());
    }
}
