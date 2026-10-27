//! IPC client to communicate with `panel-agent` over a Unix domain socket.

use panel_proto::{
    AcmeChallengeCleanedResponse, AcmeChallengeCleanupRequest, AcmeChallengeDeployRequest,
    AcmeChallengeDeployedResponse, BackupCreateRequest, BackupCreateResponse, BackupDeleteRequest,
    BackupDeleteResponse, BackupRestoreRequest, BackupRestoreResponse, CronJobApplyRequest,
    CronJobApplyResponse, CronJobLogsReadRequest, CronJobLogsReadResponse, CronJobRemoveRequest,
    CronJobRemoveResponse, CronJobRunNowRequest, CronJobRunNowResponse, DatabaseCreateRequest,
    DatabaseCreateResponse, DatabaseDeleteRequest, DatabaseDeleteResponse, DatabaseExportRequest,
    DatabaseExportResponse, DatabaseImportRequest, DatabaseImportResponse,
    DatabaseRotatePasswordRequest, DatabaseRotatePasswordResponse, DatabaseServerActionRequest,
    DatabaseServerActionResponse, DatabaseServerInstallRequest, DatabaseServerInstallResponse,
    DatabaseServerStatusRequest, DatabaseServerStatusResponse, DatabaseSizeCheckRequest,
    DatabaseSizeCheckResponse, DatabaseUpdateRemoteAccessRequest,
    DatabaseUpdateRemoteAccessResponse, FsActionResponse, FsArchiveCompressRequest,
    FsArchiveCompressResponse, FsArchiveExtractRequest, FsArchiveExtractResponse,
    FsChmodRecursiveRequest, FsChmodRecursiveResponse, FsChmodRequest, FsChmodResponse,
    FsChownRequest, FsChownResponse, FsCopyRequest, FsDeleteRequest, FsListRequest, FsListResponse,
    FsMkdirRequest, FsReadFileRequest, FsReadFileResponse, FsReadRangeRequest, FsReadRangeResponse,
    FsRenameRequest, FsSearchRequest, FsSearchResponse, FsStatRequest, FsStatResponse,
    FsTrashEmptyRequest, FsTrashEmptyResponse, FsTrashListRequest, FsTrashListResponse,
    FsTrashRequest, FsTrashResponse, FsTrashRestoreRequest, FsTrashRestoreResponse,
    FsUploadAppendChunkRequest, FsUploadAppendChunkResponse, FsUploadGetSessionRequest,
    FsUploadGetSessionResponse, FsUploadInitRequest, FsUploadInitResponse, FsWriteAtomicRequest,
    FsWriteAtomicResponse, FsWriteEditorRequest, FsZipDirectoryRequest, FsZipDirectoryResponse,
    HardeningApplyRequest, HardeningApplyResponse, HardeningStatus, NftablesStatusRequest,
    NftablesStatusResponse, NftablesSyncRequest, NftablesSyncResponse,
    NginxBaselineAppliedResponse, NginxBaselineApplyRequest, NodeRuntimeActionResponse,
    NodeRuntimeCorepackRequest, NodeRuntimeInstallRequest, NodeRuntimeListRequest,
    NodeRuntimeListResponse, NodeRuntimeRemoveRequest, NodeSiteActionRequest,
    NodeSiteActionResponse, NodeSiteApplyRequest, NodeSiteApplyResponse, NodeSiteJobRequest,
    NodeSiteJobResponse, NodeSiteRemoveRequest, NodeSiteStatusRequest, NodeSiteStatusResponse,
    OptimizerApplyRequest, OptimizerApplyResponse, OptimizerGetStatusRequest,
    OptimizerRollbackRequest, OptimizerRollbackResponse, OptimizerStatusResponse,
    PhpExtensionActionResponse, PhpExtensionInstallRequest, PhpExtensionListRequest,
    PhpExtensionListResponse, PhpExtensionRemoveRequest, PhpExtensionToggleRequest,
    PhpFpmHealthCheckRequest, PhpFpmHealthCheckResponse, PhpFpmMasterActionResponse,
    PhpFpmMasterEnsureRequest, PhpFpmMasterEnsureResponse, PhpFpmMasterReloadRequest,
    PhpFpmMasterRestartRequest, PhpFpmMasterStatusRequest, PhpFpmMasterStatusResponse,
    PhpFpmPoolApplyRequest, PhpFpmPoolApplyResponse, PhpFpmPoolRemoveRequest,
    PhpFpmPoolRemoveResponse, PhpFpmPoolStatusRequest, PhpFpmPoolStatusResponse,
    PhpLogsReadRequest, PhpLogsReadResponse, PhpRepoSetupRequest, PhpRepoSetupResponse,
    PhpRuntimeActionResponse, PhpRuntimeInstallRequest, PhpRuntimeListRequest,
    PhpRuntimeListResponse, PhpRuntimeRemoveRequest, ProtoError, ProxyUpstreamProbeRequest,
    ProxyUpstreamProbeResponse, PtyCloseRequest, PtyCloseResponse, PtyInputRequest,
    PtyInputResponse, PtyListRequest, PtyListResponse, PtyOpenRequest, PtyOpenResponse,
    PtyReadOutputRequest, PtyReadOutputResponse, PtyResizeRequest, PtyResizeResponse,
    PythonRuntimeActionResponse, PythonRuntimeInstallRequest, PythonRuntimeListRequest,
    PythonRuntimeListResponse, PythonRuntimeRemoveRequest, PythonSiteActionRequest,
    PythonSiteActionResponse, PythonSiteApplyRequest, PythonSiteApplyResponse,
    PythonSiteJobRequest, PythonSiteJobResponse, PythonSiteRemoveRequest, PythonSiteStatusRequest,
    PythonSiteStatusResponse, QuotaGetRequest, QuotaGetResponse, QuotaScanRequest,
    QuotaScanResponse, QuotaSetRequest, QuotaSetResponse, RequestPayload, ResourceReadRequest,
    ResponsePayload, RpcRequest, RpcStream, SiteAppliedResponse, SiteApplyRequest,
    SiteDeployKeyGenerateRequest, SiteDeployKeyGenerateResponse, SiteEnvApplyRequest,
    SiteEnvApplyResponse, SiteEnvDecryptRequest, SiteEnvDecryptResponse, SiteEnvEncryptRequest,
    SiteEnvEncryptResponse, SiteGitDeployRequest, SiteGitDeployResponse,
    SiteGitReleasesListRequest, SiteGitReleasesListResponse, SiteGitRollbackRequest,
    SiteGitRollbackResponse, SiteHtpasswdDeployRequest, SiteHtpasswdDeployedResponse,
    SiteHtpasswdRemoveRequest, SiteHtpasswdRemovedResponse, SiteLogsReadRequest,
    SiteLogsReadResponse, SiteProvisionRequest, SiteProvisionResponse, SiteRemoveRequest,
    SiteRemovedResponse, SliceAppliedResponse, SliceApplyRequest, SliceRemoveRequest,
    SliceRemovedResponse, SliceResourceMetrics, SshDropinApplyRequest, SshDropinApplyResponse,
    SshKeysSyncRequest, SshKeysSyncResponse, SshPolicyApplyRequest, SshPolicyApplyResponse,
    SslCertDeployRequest, SslCertDeployedResponse, SystemBackupCreateRequest,
    SystemBackupCreateResponse, SystemBackupListRequest, SystemBackupListResponse,
    SystemBackupRestoreRequest, SystemBackupRestoreResponse, SystemDoctorCheckRequest,
    SystemDoctorCheckResponse, SystemDoctorFixRequest, SystemDoctorFixResponse,
    SystemReconcileRequest, SystemReconcileResponse, UserCreateRequest, UserCreatedResponse,
    UserDeleteRequest, UserDeletedResponse, UserSuspendRequest, UserSuspendedResponse,
    UserUnsuspendRequest, UserUnsuspendedResponse, VersionInfo,
};
use std::path::PathBuf;
use thiserror::Error;

#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;
#[cfg(not(unix))]
type IpcStream = tokio::net::TcpStream;

#[derive(Debug, Error)]
pub enum AgentClientError {
    #[error("Failed to connect to agent socket at {path}: {source}")]
    Connect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Protocol error: {0}")]
    Proto(#[from] ProtoError),
    #[error("Agent returned error: {0}")]
    AgentError(String),
    #[error("Unexpected agent response: expected {expected}, got {got}")]
    UnexpectedResponse { expected: &'static str, got: String },
}

#[derive(Debug, Clone)]
pub struct AgentClient {
    socket_path: PathBuf,
}

impl AgentClient {
    pub async fn server_backup(
        &self,
        operation: panel_proto::ServerBackupRequest,
    ) -> Result<serde_json::Value, AgentClientError> {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(25),
            self.send_rpc("server-backup", RequestPayload::ServerBackup(operation)),
        )
        .await
        .map_err(|_| {
            AgentClientError::AgentError("Periksa status backup sebelum mencoba ulang.".into())
        })??;
        if !response.success {
            return Err(AgentClientError::AgentError(
                response
                    .error
                    .unwrap_or_else(|| "Backup belum tersedia.".into()),
            ));
        }
        match response.payload {
            ResponsePayload::ServerBackup(value) => Ok(value),
            _ => Err(AgentClientError::AgentError(
                "Respons backup tidak sesuai.".into(),
            )),
        }
    }
    pub async fn panel_update(
        &self,
        operation: panel_proto::PanelUpdateRequest,
    ) -> Result<panel_proto::PanelUpdateStatus, AgentClientError> {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(25),
            self.send_rpc("panel-update", RequestPayload::PanelUpdate(operation)),
        )
        .await
        .map_err(|_| {
            AgentClientError::AgentError(
                "Status update belum diterima; periksa kembali sebelum mencoba ulang.".into(),
            )
        })??;
        if !response.success {
            return Err(AgentClientError::AgentError(
                response
                    .error
                    .unwrap_or_else(|| "Operasi updater gagal.".into()),
            ));
        }
        match response.payload {
            ResponsePayload::PanelUpdate(status) => Ok(status),
            _ => Err(AgentClientError::AgentError(
                "Respons updater tidak sesuai.".into(),
            )),
        }
    }
    pub async fn dns01(
        &self,
        command: panel_proto::Dns01Request,
    ) -> Result<serde_json::Value, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req = RpcRequest::new(
            uuid::Uuid::new_v4().to_string(),
            RequestPayload::Dns01(command),
        );
        rpc.send_request(&req).await?;
        let res = tokio::time::timeout(std::time::Duration::from_secs(180), rpc.recv_response())
            .await
            .map_err(|_| {
                AgentClientError::AgentError("Batas waktu DNS-01 habis; periksa cleanup.".into())
            })??;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error.unwrap_or_else(|| "Operasi DNS-01 gagal.".into()),
            ));
        }
        match res.payload {
            ResponsePayload::Dns01(value) => Ok(value),
            _ => Err(AgentClientError::AgentError(
                "Respons DNS-01 tidak sesuai.".into(),
            )),
        }
    }
    pub async fn panel_tls(
        &self,
        command: panel_proto::PanelTlsRequest,
    ) -> Result<panel_proto::PanelTlsStatus, AgentClientError> {
        let mut rpc = self.connect().await?;
        let request = RpcRequest::new(
            uuid::Uuid::new_v4().to_string(),
            RequestPayload::PanelTls(command),
        );
        rpc.send_request(&request).await?;
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(60), rpc.recv_response())
                .await
                .map_err(|_| {
                    AgentClientError::AgentError("Batas waktu operasi TLS panel habis.".into())
                })??;
        if !response.success {
            return Err(AgentClientError::AgentError(
                "Operasi TLS panel gagal; periksa status kandidat dan Nginx.".into(),
            ));
        }
        match response.payload {
            ResponsePayload::PanelTls(status) => Ok(status),
            _ => Err(AgentClientError::AgentError(
                "Respons TLS panel tidak sesuai.".into(),
            )),
        }
    }
    pub async fn ssh_host_keys(
        &self,
    ) -> Result<Vec<panel_core::ssh::SshHostKeyFingerprint>, AgentClientError> {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let response = self
                .send_rpc("ssh-host-keys", RequestPayload::SshHostKeys)
                .await?;
            match response.payload {
                ResponsePayload::SshHostKeys(keys) if response.success && keys.len() <= 3 => {
                    Ok(keys)
                }
                _ => Err(AgentClientError::AgentError(
                    "Identitas OpenSSH tidak tersedia.".into(),
                )),
            }
        })
        .await
        .map_err(|_| {
            AgentClientError::AgentError("Pemeriksaan identitas OpenSSH timeout.".into())
        })?
    }

    pub async fn connectivity_inspect(
        &self,
        target: panel_proto::ConnectivityTarget,
    ) -> Result<panel_proto::ConnectivityInspection, AgentClientError> {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut rpc = self.connect().await?;
            rpc.send_request(&RpcRequest::new(
                uuid::Uuid::new_v4().to_string(),
                RequestPayload::ConnectivityInspect(target),
            ))
            .await?;
            let response = rpc.recv_response().await?;
            match response.payload {
                ResponsePayload::ConnectivityInspected(result) if response.success => Ok(result),
                _ => Err(AgentClientError::AgentError(
                    "Diagnosis lokal tidak tersedia".into(),
                )),
            }
        })
        .await
        .map_err(|_| AgentClientError::AgentError("Diagnosis lokal timeout".into()))?
    }

    pub async fn database_web(
        &self,
        request: RequestPayload,
    ) -> Result<ResponsePayload, AgentClientError> {
        if !matches!(
            request,
            RequestPayload::DatabaseWebStatus | RequestPayload::DatabaseWebCredentials(_)
        ) {
            return Err(AgentClientError::AgentError(
                "Permintaan pengelola database tidak valid".into(),
            ));
        }
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut rpc = self.connect().await?;
            rpc.send_request(&RpcRequest::new(uuid::Uuid::new_v4().to_string(), request))
                .await?;
            let response = rpc.recv_response().await?;
            if !response.success {
                return Err(AgentClientError::AgentError(
                    "Agent gagal menyiapkan pengelola database".into(),
                ));
            }
            Ok(response.payload)
        })
        .await
        .map_err(|_| AgentClientError::AgentError("Pengelola database timeout".into()))?
    }
    pub async fn ai_secret(
        &self,
        request: panel_proto::AiSecretRequest,
    ) -> Result<panel_proto::AiSecret, AgentClientError> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut rpc = self.connect().await?;
            rpc.send_request(&RpcRequest::new(
                uuid::Uuid::new_v4().to_string(),
                RequestPayload::AiSecret(request),
            ))
            .await?;
            let response = rpc.recv_response().await?;
            match (response.success, response.payload) {
                (true, ResponsePayload::AiSecret(value)) => Ok(value),
                _ => Err(AgentClientError::AgentError(
                    "Secret AI belum tersedia. Perbarui paket API/agent lengkap.".into(),
                )),
            }
        })
        .await
        .map_err(|_| AgentClientError::AgentError("Pemrosesan secret AI timeout.".into()))?
    }

    pub async fn managed_storage(
        &self,
        request: panel_proto::ManagedStorageRequest,
    ) -> Result<serde_json::Value, AgentClientError> {
        tokio::time::timeout(std::time::Duration::from_secs(480), async {
            let mut rpc = self.connect().await?;
            rpc.send_request(&RpcRequest::new(
                uuid::Uuid::new_v4().to_string(),
                RequestPayload::ManagedStorage(request),
            ))
            .await?;
            let response = rpc.recv_response().await?;
            match (response.success, response.payload) {
                (true, ResponsePayload::ManagedStorage(value)) => Ok(value),
                _ => Err(AgentClientError::AgentError(
                    "Layanan storage belum tersedia. Periksa paket API/agent dan journal layanan."
                        .into(),
                )),
            }
        })
        .await
        .map_err(|_| {
            AgentClientError::AgentError(
                "Operasi storage terputus. Muat ulang status sebelum mencoba kembali.".into(),
            )
        })?
    }

    pub async fn storage_secret(
        &self,
        request: panel_proto::StorageSecretRequest,
    ) -> Result<panel_proto::StorageSecret, AgentClientError> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut rpc = self.connect().await?;
            rpc.send_request(&RpcRequest::new(
                uuid::Uuid::new_v4().to_string(),
                RequestPayload::StorageSecret(request),
            ))
            .await?;
            let response = rpc.recv_response().await?;
            match (response.success, response.payload) {
                (true, ResponsePayload::StorageSecret(value)) => Ok(value),
                _ => Err(AgentClientError::AgentError(
                    "Credential storage belum tersedia. Perbarui paket API/agent lengkap.".into(),
                )),
            }
        })
        .await
        .map_err(|_| {
            AgentClientError::AgentError("Pemrosesan credential storage timeout.".into())
        })?
    }

    pub async fn email(
        &self,
        request: panel_proto::EmailRequest,
    ) -> Result<serde_json::Value, AgentClientError> {
        tokio::time::timeout(std::time::Duration::from_secs(45), async {
            let mut rpc = self.connect().await?;
            rpc.send_request(&RpcRequest::new(
                uuid::Uuid::new_v4().to_string(),
                RequestPayload::Email(request),
            ))
            .await?;
            let response = rpc.recv_response().await?;
            if !response.success {
                return Err(AgentClientError::AgentError(
                    response
                        .error
                        .unwrap_or_else(|| "Operasi Email gagal.".into()),
                ));
            }
            match response.payload {
                ResponsePayload::Email(value) => Ok(value),
                _ => Err(AgentClientError::UnexpectedResponse {
                    expected: "Email",
                    got: "unexpected payload".into(),
                }),
            }
        })
        .await
        .map_err(|_| {
            AgentClientError::AgentError(
                "Operasi email timeout. Periksa status sebelum mencoba kembali.".into(),
            )
        })?
    }
    pub async fn cloudflare(
        &self,
        request: panel_proto::CloudflareRequest,
    ) -> Result<serde_json::Value, AgentClientError> {
        let mut rpc = self.connect().await?;
        rpc.send_request(&RpcRequest::new(
            uuid::Uuid::new_v4().to_string(),
            RequestPayload::Cloudflare(request),
        ))
        .await?;
        let response = rpc.recv_response().await?;
        if !response.success {
            return Err(AgentClientError::AgentError(
                response
                    .error
                    .unwrap_or_else(|| "Operasi Cloudflare gagal.".into()),
            ));
        }
        match response.payload {
            ResponsePayload::Cloudflare(value) => Ok(value),
            _ => Err(AgentClientError::UnexpectedResponse {
                expected: "Cloudflare",
                got: "unexpected payload".into(),
            }),
        }
    }
    /// Read bounded progress frames until the typed installation result arrives.
    pub async fn install_with_progress<F, Fut>(
        &self,
        operation: panel_proto::InstallOperation,
        mut on_log: F,
    ) -> Result<ResponsePayload, AgentClientError>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let mut rpc = self.connect().await?;
        let id = uuid::Uuid::new_v4().to_string();
        rpc.send_request(&RpcRequest::new(
            &id,
            RequestPayload::InstallWithProgress(operation),
        ))
        .await?;
        loop {
            let response = rpc.recv_response().await?;
            if response.request_id != id {
                return Err(AgentClientError::AgentError(
                    "Installation response ID mismatch".into(),
                ));
            }
            if !response.success {
                return Err(AgentClientError::AgentError(
                    response.error.unwrap_or_else(|| "Instalasi gagal".into()),
                ));
            }
            match response.payload {
                ResponsePayload::InstallationProgress(event) => on_log(event.message)
                    .await
                    .map_err(AgentClientError::AgentError)?,
                result => return Ok(result),
            }
        }
    }
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    async fn connect(&self) -> Result<RpcStream<IpcStream>, AgentClientError> {
        #[cfg(unix)]
        {
            let stream = tokio::net::UnixStream::connect(&self.socket_path)
                .await
                .map_err(|e| AgentClientError::Connect {
                    path: self.socket_path.clone(),
                    source: e,
                })?;
            Ok(RpcStream::new(stream))
        }
        #[cfg(not(unix))]
        {
            let port_str = if self.socket_path.exists() {
                std::fs::read_to_string(&self.socket_path).unwrap_or_else(|_| "7701".to_string())
            } else {
                "7701".to_string()
            };
            let port: u16 = port_str.trim().parse().unwrap_or(7701);
            let addr = format!("127.0.0.1:{}", port);
            let stream = tokio::net::TcpStream::connect(&addr).await.map_err(|e| {
                AgentClientError::Connect {
                    path: self.socket_path.clone(),
                    source: e,
                }
            })?;
            Ok(RpcStream::new(stream))
        }
    }

    pub async fn ping(&self) -> Result<(), AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "ping-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let req = RpcRequest::new(req_id, RequestPayload::Ping);
        rpc.send_request(&req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::Pong => Ok(()),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "Pong",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn version(&self) -> Result<VersionInfo, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "ver-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let req = RpcRequest::new(req_id, RequestPayload::Version);
        rpc.send_request(&req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::Version(info) => Ok(info),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "Version",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn create_user(
        &self,
        username: impl Into<String>,
        uid: u32,
        shell: Option<String>,
        home_dir: Option<String>,
        web_user: Option<String>,
    ) -> Result<UserCreatedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let username_str = username.into();
        let req_id = format!(
            "ucreate-{}-{}",
            username_str,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let req = RpcRequest::new(
            req_id,
            RequestPayload::UserCreate(UserCreateRequest {
                username: username_str,
                uid,
                shell,
                home_dir,
                web_user,
            }),
        );
        rpc.send_request(&req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::UserCreated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "UserCreated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn suspend_user(
        &self,
        username: impl Into<String>,
        uid: u32,
    ) -> Result<UserSuspendedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let username_str = username.into();
        let req_id = format!(
            "ususpend-{}-{}",
            username_str,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let req = RpcRequest::new(
            req_id,
            RequestPayload::UserSuspend(UserSuspendRequest {
                username: username_str,
                uid,
            }),
        );
        rpc.send_request(&req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::UserSuspended(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "UserSuspended",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn unsuspend_user(
        &self,
        username: impl Into<String>,
        uid: u32,
        shell: Option<String>,
    ) -> Result<UserUnsuspendedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let username_str = username.into();
        let req_id = format!(
            "uunsuspend-{}-{}",
            username_str,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let req = RpcRequest::new(
            req_id,
            RequestPayload::UserUnsuspend(UserUnsuspendRequest {
                username: username_str,
                uid,
                shell,
            }),
        );
        rpc.send_request(&req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::UserUnsuspended(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "UserUnsuspended",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn delete_user(
        &self,
        username: impl Into<String>,
        uid: u32,
        purge: bool,
        trash_dir: Option<String>,
    ) -> Result<UserDeletedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let username_str = username.into();
        let req_id = format!(
            "udelete-{}-{}",
            username_str,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let req = RpcRequest::new(
            req_id,
            RequestPayload::UserDelete(UserDeleteRequest {
                username: username_str,
                uid,
                purge,
                trash_dir,
            }),
        );
        rpc.send_request(&req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::UserDeleted(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "UserDeleted",
                got: format!("{other:?}"),
            }),
        }
    }
    pub async fn apply_slice(
        &self,
        req: SliceApplyRequest,
    ) -> Result<SliceAppliedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "slice-apply-{}-{}",
            req.uid,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SliceApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SliceApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SliceApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_slice(&self, uid: u32) -> Result<SliceRemovedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "slice-remove-{}-{}",
            uid,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::SliceRemove(SliceRemoveRequest { uid }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SliceRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SliceRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn set_quota(
        &self,
        req: QuotaSetRequest,
    ) -> Result<QuotaSetResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "quota-set-{}-{}",
            req.uid,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::QuotaSet(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::QuotaSet(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "QuotaSet",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn get_quota(
        &self,
        req: QuotaGetRequest,
    ) -> Result<QuotaGetResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "quota-get-{}-{}",
            req.uid,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::QuotaGet(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::QuotaGet(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "QuotaGet",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn scan_quota(
        &self,
        req: QuotaScanRequest,
    ) -> Result<QuotaScanResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "quota-scan-{}-{}",
            req.uid,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::QuotaScan(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::QuotaScan(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "QuotaScan",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn read_slice_resources(
        &self,
        uid: u32,
    ) -> Result<SliceResourceMetrics, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "resource-read-{}-{}",
            uid,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::ResourceRead(ResourceReadRequest { uid }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::ResourceRead(data) => Ok(data.metrics),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "ResourceRead",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn check_hardening(&self) -> Result<HardeningStatus, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "hardening-check-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::HardeningCheck);
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::HardeningStatus(data) => Ok(*data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "HardeningStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_hardening(
        &self,
        req: HardeningApplyRequest,
    ) -> Result<HardeningApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "hardening-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::HardeningApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::HardeningApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "HardeningApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_nginx_baseline(
        &self,
        req: NginxBaselineApplyRequest,
    ) -> Result<NginxBaselineAppliedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "nginx-baseline-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NginxBaselineApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NginxBaselineApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NginxBaselineApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_site(
        &self,
        req: SiteApplyRequest,
    ) -> Result<SiteAppliedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-apply-{}-{}",
            req.site_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_site(
        &self,
        req: SiteRemoveRequest,
    ) -> Result<SiteRemovedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-remove-{}-{}",
            req.site_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn deploy_site_htpasswd(
        &self,
        req: SiteHtpasswdDeployRequest,
    ) -> Result<SiteHtpasswdDeployedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-htpasswd-deploy-{}-{}",
            req.site_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteHtpasswdDeploy(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteHtpasswdDeployed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteHtpasswdDeployed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_site_htpasswd(
        &self,
        req: SiteHtpasswdRemoveRequest,
    ) -> Result<SiteHtpasswdRemovedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-htpasswd-remove-{}-{}",
            req.site_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteHtpasswdRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteHtpasswdRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteHtpasswdRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn provision_site(
        &self,
        req: SiteProvisionRequest,
    ) -> Result<SiteProvisionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-provision-{}-{}",
            req.site_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteProvision(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteProvisioned(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteProvisioned",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn deploy_acme_challenge(
        &self,
        req: AcmeChallengeDeployRequest,
    ) -> Result<AcmeChallengeDeployedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "acme-deploy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::AcmeChallengeDeploy(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::AcmeChallengeDeployed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "AcmeChallengeDeployed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn cleanup_acme_challenge(
        &self,
        req: AcmeChallengeCleanupRequest,
    ) -> Result<AcmeChallengeCleanedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "acme-cleanup-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::AcmeChallengeCleanup(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::AcmeChallengeCleaned(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "AcmeChallengeCleaned",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn deploy_ssl_cert(
        &self,
        req: SslCertDeployRequest,
    ) -> Result<SslCertDeployedResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "ssl-deploy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SslCertDeploy(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SslCertDeployed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SslCertDeployed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn read_site_logs(
        &self,
        req: SiteLogsReadRequest,
    ) -> Result<SiteLogsReadResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-logs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteLogsRead(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteLogsRead(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteLogsRead",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn setup_php_repo(&self) -> Result<PhpRepoSetupResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-repo-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PhpRepoSetup(PhpRepoSetupRequest {}));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpRepoSetup(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpRepoSetup",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn list_php_runtimes(&self) -> Result<PhpRuntimeListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpRuntimeList(PhpRuntimeListRequest {}),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpRuntimeList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpRuntimeList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn install_php_runtime(
        &self,
        version: impl Into<String>,
    ) -> Result<PhpRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-install-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpRuntimeInstall(PhpRuntimeInstallRequest {
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpRuntimeInstalled(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpRuntimeInstalled",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_php_runtime(
        &self,
        version: impl Into<String>,
    ) -> Result<PhpRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-remove-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpRuntimeRemove(PhpRuntimeRemoveRequest {
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpRuntimeRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpRuntimeRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn list_php_extensions(
        &self,
        version: impl Into<String>,
    ) -> Result<PhpExtensionListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-ext-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpExtensionList(PhpExtensionListRequest {
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpExtensionList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpExtensionList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn install_php_extension(
        &self,
        version: impl Into<String>,
        extension: impl Into<String>,
    ) -> Result<PhpExtensionActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-ext-install-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpExtensionInstall(PhpExtensionInstallRequest {
                version: version.into(),
                extension: extension.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpExtensionAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpExtensionAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn toggle_php_extension(
        &self,
        version: impl Into<String>,
        extension: impl Into<String>,
        enabled: bool,
    ) -> Result<PhpExtensionActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-ext-toggle-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpExtensionToggle(PhpExtensionToggleRequest {
                version: version.into(),
                extension: extension.into(),
                enabled,
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpExtensionAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpExtensionAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_php_extension(
        &self,
        version: impl Into<String>,
        extension: impl Into<String>,
    ) -> Result<PhpExtensionActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-ext-remove-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpExtensionRemove(PhpExtensionRemoveRequest {
                version: version.into(),
                extension: extension.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpExtensionAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpExtensionAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn ensure_php_fpm_master(
        &self,
        username: impl Into<String>,
        uid: u32,
        version: impl Into<String>,
    ) -> Result<PhpFpmMasterEnsureResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-ensure-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmMasterEnsure(PhpFpmMasterEnsureRequest {
                username: username.into(),
                uid,
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmMasterEnsure(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmMasterEnsure",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_php_fpm_pool(
        &self,
        req: PhpFpmPoolApplyRequest,
    ) -> Result<PhpFpmPoolApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-pool-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PhpFpmPoolApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmPoolApply(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmPoolApply",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_php_fpm_pool(
        &self,
        username: impl Into<String>,
        version: impl Into<String>,
        pool_name: impl Into<String>,
        reload: bool,
    ) -> Result<PhpFpmPoolRemoveResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-pool-rem-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmPoolRemove(PhpFpmPoolRemoveRequest {
                username: username.into(),
                version: version.into(),
                pool_name: pool_name.into(),
                reload,
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmPoolRemove(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmPoolRemove",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn get_php_fpm_master_status(
        &self,
        username: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<PhpFpmMasterStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmMasterStatus(PhpFpmMasterStatusRequest {
                username: username.into(),
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmMasterStatus(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmMasterStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn reload_php_fpm_master(
        &self,
        username: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<PhpFpmMasterActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-reload-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmMasterReload(PhpFpmMasterReloadRequest {
                username: username.into(),
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmMasterAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmMasterAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn restart_php_fpm_master(
        &self,
        username: impl Into<String>,
        version: impl Into<String>,
    ) -> Result<PhpFpmMasterActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-restart-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmMasterRestart(PhpFpmMasterRestartRequest {
                username: username.into(),
                version: version.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmMasterAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmMasterAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn check_php_fpm_health(
        &self,
        username: impl Into<String>,
        version: impl Into<String>,
        pool_name: impl Into<String>,
    ) -> Result<PhpFpmHealthCheckResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-health-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmHealthCheck(PhpFpmHealthCheckRequest {
                username: username.into(),
                version: version.into(),
                pool_name: pool_name.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmHealthChecked(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmHealthChecked",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn get_php_fpm_pool_status(
        &self,
        username: impl Into<String>,
        version: impl Into<String>,
        pool_name: impl Into<String>,
    ) -> Result<PhpFpmPoolStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-fpm-pool-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::PhpFpmPoolStatus(PhpFpmPoolStatusRequest {
                username: username.into(),
                version: version.into(),
                pool_name: pool_name.into(),
            }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpFpmPoolStatus(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpFpmPoolStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn read_php_logs(
        &self,
        req: PhpLogsReadRequest,
    ) -> Result<PhpLogsReadResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "php-logs-read-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PhpLogsRead(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PhpLogsRead(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PhpLogsRead",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_list(&self, req: FsListRequest) -> Result<FsListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsList(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_stat(&self, req: FsStatRequest) -> Result<FsStatResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-stat-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsStat(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsStat(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsStat",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_mkdir(
        &self,
        req: FsMkdirRequest,
    ) -> Result<FsActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-mkdir-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsMkdir(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_rename(
        &self,
        req: FsRenameRequest,
    ) -> Result<FsActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-rename-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsRename(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_copy(&self, req: FsCopyRequest) -> Result<FsActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-copy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsCopy(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_trash(&self, req: FsTrashRequest) -> Result<FsTrashResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-trash-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsTrash(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsTrash(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsTrash",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_trash_list(
        &self,
        req: FsTrashListRequest,
    ) -> Result<FsTrashListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-trash-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsTrashList(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsTrashList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsTrashList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_trash_restore(
        &self,
        req: FsTrashRestoreRequest,
    ) -> Result<FsTrashRestoreResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-trash-restore-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsTrashRestore(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsTrashRestore(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsTrashRestore",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_trash_empty(
        &self,
        req: FsTrashEmptyRequest,
    ) -> Result<FsTrashEmptyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-trash-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsTrashEmpty(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsTrashEmpty(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsTrashEmpty",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_delete(
        &self,
        req: FsDeleteRequest,
    ) -> Result<FsActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-delete-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsDelete(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_upload_init(
        &self,
        req: FsUploadInitRequest,
    ) -> Result<FsUploadInitResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-upload-init-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsUploadInit(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsUploadInit(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsUploadInit",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_upload_get_session(
        &self,
        req: FsUploadGetSessionRequest,
    ) -> Result<FsUploadGetSessionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-upload-get-session-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsUploadGetSession(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsUploadGetSession(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsUploadGetSession",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_upload_append_chunk(
        &self,
        req: FsUploadAppendChunkRequest,
    ) -> Result<FsUploadAppendChunkResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-upload-append-chunk-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsUploadAppendChunk(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsUploadAppendChunk(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsUploadAppendChunk",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_read_range(
        &self,
        req: FsReadRangeRequest,
    ) -> Result<FsReadRangeResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-read-range-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsReadRange(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsReadRange(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsReadRange",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_zip_directory(
        &self,
        req: FsZipDirectoryRequest,
    ) -> Result<FsZipDirectoryResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-zip-directory-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsZipDirectory(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsZipDirectory(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsZipDirectory",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_archive_compress(
        &self,
        req: FsArchiveCompressRequest,
    ) -> Result<FsArchiveCompressResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-archive-compress-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsArchiveCompress(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsArchiveCompress(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsArchiveCompress",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_archive_extract(
        &self,
        req: FsArchiveExtractRequest,
    ) -> Result<FsArchiveExtractResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-archive-extract-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsArchiveExtract(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsArchiveExtract(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsArchiveExtract",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_chmod(&self, req: FsChmodRequest) -> Result<FsChmodResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-chmod-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsChmod(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsChmod(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsChmod",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_chmod_recursive(
        &self,
        req: FsChmodRecursiveRequest,
    ) -> Result<FsChmodRecursiveResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-chmod-recursive-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsChmodRecursive(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsChmodRecursive(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsChmodRecursive",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_chown(&self, req: FsChownRequest) -> Result<FsChownResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-chown-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsChown(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsChown(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsChown",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_read_file(
        &self,
        req: FsReadFileRequest,
    ) -> Result<FsReadFileResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-read-file-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsReadFile(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsReadFile(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsReadFile",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_write_atomic(
        &self,
        req: FsWriteAtomicRequest,
    ) -> Result<FsWriteAtomicResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-write-atomic-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsWriteAtomic(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsWriteAtomic(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsWriteAtomic",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_write_editor(
        &self,
        req: FsWriteEditorRequest,
    ) -> Result<FsWriteAtomicResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-write-editor-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsWriteEditor(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsWriteEditor(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsWriteEditor",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn fs_search(
        &self,
        req: FsSearchRequest,
    ) -> Result<FsSearchResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "fs-search-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::FsSearch(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::FsSearch(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "FsSearch",
                got: format!("{other:?}"),
            }),
        }
    }

    async fn send_rpc(
        &self,
        prefix: &str,
        payload: RequestPayload,
    ) -> Result<panel_proto::RpcResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, payload);
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        Ok(res)
    }

    pub async fn pty_open(&self, req: PtyOpenRequest) -> Result<PtyOpenResponse, AgentClientError> {
        let res = self
            .send_rpc("pty-open", RequestPayload::PtyOpen(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PtyOpen(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PtyOpen",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn pty_input(
        &self,
        req: PtyInputRequest,
    ) -> Result<PtyInputResponse, AgentClientError> {
        let res = self
            .send_rpc("pty-in", RequestPayload::PtyInput(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PtyInput(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PtyInput",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn pty_read_output(
        &self,
        req: PtyReadOutputRequest,
    ) -> Result<PtyReadOutputResponse, AgentClientError> {
        let res = self
            .send_rpc("pty-read", RequestPayload::PtyReadOutput(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PtyReadOutput(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PtyReadOutput",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn pty_resize(
        &self,
        req: PtyResizeRequest,
    ) -> Result<PtyResizeResponse, AgentClientError> {
        let res = self
            .send_rpc("pty-res", RequestPayload::PtyResize(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PtyResize(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PtyResize",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn pty_close(
        &self,
        req: PtyCloseRequest,
    ) -> Result<PtyCloseResponse, AgentClientError> {
        let res = self
            .send_rpc("pty-close", RequestPayload::PtyClose(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PtyClose(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PtyClose",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn pty_list(&self, req: PtyListRequest) -> Result<PtyListResponse, AgentClientError> {
        let res = self
            .send_rpc("pty-list", RequestPayload::PtyList(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PtyList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PtyList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn sync_ssh_keys(
        &self,
        linux_username: &str,
        keys: Vec<String>,
    ) -> Result<SshKeysSyncResponse, AgentClientError> {
        let req = SshKeysSyncRequest {
            linux_username: linux_username.to_string(),
            keys,
        };
        let res = self
            .send_rpc("ssh-keys-sync", RequestPayload::SshKeysSync(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SshKeysSync(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SshKeysSync",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_ssh_policy(
        &self,
        linux_username: &str,
        uid: u32,
        policy: &str,
    ) -> Result<SshPolicyApplyResponse, AgentClientError> {
        let req = SshPolicyApplyRequest {
            linux_username: linux_username.to_string(),
            uid,
            policy: policy.to_string(),
        };
        let res = self
            .send_rpc("ssh-policy-apply", RequestPayload::SshPolicyApply(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SshPolicyApply(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SshPolicyApply",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_ssh_dropin(
        &self,
        custom_content: Option<String>,
    ) -> Result<SshDropinApplyResponse, AgentClientError> {
        let req = SshDropinApplyRequest { custom_content };
        let res = self
            .send_rpc("ssh-dropin-apply", RequestPayload::SshDropinApply(req))
            .await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SshDropinApply(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SshDropinApply",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn list_node_runtimes(
        &self,
        base_dir: Option<String>,
    ) -> Result<NodeRuntimeListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::NodeRuntimeList(NodeRuntimeListRequest { base_dir }),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeRuntimeList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeRuntimeList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn install_node_runtime(
        &self,
        req: NodeRuntimeInstallRequest,
    ) -> Result<NodeRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-install-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeRuntimeInstall(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeRuntimeInstalled(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeRuntimeInstalled",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_node_runtime(
        &self,
        req: NodeRuntimeRemoveRequest,
    ) -> Result<NodeRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-remove-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeRuntimeRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeRuntimeRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeRuntimeRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn enable_node_corepack(
        &self,
        req: NodeRuntimeCorepackRequest,
    ) -> Result<NodeRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-cp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeRuntimeCorepack(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeRuntimeCorepack(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeRuntimeCorepack",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_node_site(
        &self,
        req: NodeSiteApplyRequest,
    ) -> Result<NodeSiteApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-site-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeSiteApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeSiteApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeSiteApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_node_site(
        &self,
        req: NodeSiteRemoveRequest,
    ) -> Result<NodeSiteActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-site-remove-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeSiteRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeSiteRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeSiteRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn action_node_site(
        &self,
        req: NodeSiteActionRequest,
    ) -> Result<NodeSiteActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-site-action-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeSiteAction(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeSiteAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeSiteAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn status_node_site(
        &self,
        req: NodeSiteStatusRequest,
    ) -> Result<NodeSiteStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-site-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeSiteStatus(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeSiteStatus(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeSiteStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn run_node_site_job(
        &self,
        req: NodeSiteJobRequest,
    ) -> Result<NodeSiteJobResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "node-site-job-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NodeSiteJob(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NodeSiteJob(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NodeSiteJob",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn list_python_runtimes(
        &self,
        req: PythonRuntimeListRequest,
    ) -> Result<PythonRuntimeListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-runtime-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonRuntimeList(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonRuntimeList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonRuntimeList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn install_python_runtime(
        &self,
        req: PythonRuntimeInstallRequest,
    ) -> Result<PythonRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-runtime-install-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonRuntimeInstall(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonRuntimeInstalled(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonRuntimeInstalled",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_python_runtime(
        &self,
        req: PythonRuntimeRemoveRequest,
    ) -> Result<PythonRuntimeActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-runtime-remove-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonRuntimeRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonRuntimeRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonRuntimeRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_python_site(
        &self,
        req: PythonSiteApplyRequest,
    ) -> Result<PythonSiteApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-site-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonSiteApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonSiteApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonSiteApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_python_site(
        &self,
        req: PythonSiteRemoveRequest,
    ) -> Result<PythonSiteActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-site-remove-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonSiteRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonSiteRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonSiteRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn action_python_site(
        &self,
        req: PythonSiteActionRequest,
    ) -> Result<PythonSiteActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-site-action-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonSiteAction(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonSiteAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonSiteAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn status_python_site(
        &self,
        req: PythonSiteStatusRequest,
    ) -> Result<PythonSiteStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-site-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonSiteStatus(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonSiteStatus(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonSiteStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn run_python_site_job(
        &self,
        req: PythonSiteJobRequest,
    ) -> Result<PythonSiteJobResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "py-site-job-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::PythonSiteJob(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::PythonSiteJob(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "PythonSiteJob",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn probe_proxy_upstream(
        &self,
        req: ProxyUpstreamProbeRequest,
    ) -> Result<ProxyUpstreamProbeResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "proxy-probe-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::ProxyUpstreamProbe(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::ProxyUpstreamProbed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "ProxyUpstreamProbed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn encrypt_site_env(
        &self,
        req: SiteEnvEncryptRequest,
    ) -> Result<SiteEnvEncryptResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-env-enc-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteEnvEncrypt(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteEnvEncrypted(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteEnvEncrypted",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn decrypt_site_env(
        &self,
        req: SiteEnvDecryptRequest,
    ) -> Result<SiteEnvDecryptResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-env-dec-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteEnvDecrypt(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteEnvDecrypted(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteEnvDecrypted",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_site_env(
        &self,
        req: SiteEnvApplyRequest,
    ) -> Result<SiteEnvApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "site-env-app-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteEnvApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteEnvApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteEnvApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn apply_cron_job(
        &self,
        req: CronJobApplyRequest,
    ) -> Result<CronJobApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "cron-app-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::CronJobApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::CronJobApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "CronJobApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn remove_cron_job(
        &self,
        req: CronJobRemoveRequest,
    ) -> Result<CronJobRemoveResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "cron-rem-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::CronJobRemove(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::CronJobRemoved(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "CronJobRemoved",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn run_now_cron_job(
        &self,
        req: CronJobRunNowRequest,
    ) -> Result<CronJobRunNowResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "cron-run-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::CronJobRunNow(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::CronJobRunNow(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "CronJobRunNow",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn read_cron_job_logs(
        &self,
        req: CronJobLogsReadRequest,
    ) -> Result<CronJobLogsReadResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "cron-logs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::CronJobLogsRead(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::CronJobLogsRead(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "CronJobLogsRead",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn generate_deploy_key(
        &self,
        req: SiteDeployKeyGenerateRequest,
    ) -> Result<SiteDeployKeyGenerateResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "git-key-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteDeployKeyGenerate(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteDeployKeyGenerated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteDeployKeyGenerated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn git_deploy(
        &self,
        req: SiteGitDeployRequest,
    ) -> Result<SiteGitDeployResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "git-deploy-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteGitDeploy(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteGitDeployed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteGitDeployed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn git_rollback(
        &self,
        req: SiteGitRollbackRequest,
    ) -> Result<SiteGitRollbackResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "git-rollback-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteGitRollback(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteGitRollbacked(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteGitRollbacked",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn list_git_releases(
        &self,
        req: SiteGitReleasesListRequest,
    ) -> Result<SiteGitReleasesListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "git-releases-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SiteGitReleasesList(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SiteGitReleasesList(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SiteGitReleasesList",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_server_status(
        &self,
        req: DatabaseServerStatusRequest,
    ) -> Result<DatabaseServerStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-srv-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseServerStatus(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseServerStatus(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseServerStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_server_install(
        &self,
        req: DatabaseServerInstallRequest,
    ) -> Result<DatabaseServerInstallResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-srv-inst-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseServerInstall(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseServerInstalled(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseServerInstalled",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_server_action(
        &self,
        req: DatabaseServerActionRequest,
    ) -> Result<DatabaseServerActionResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-srv-act-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseServerAction(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseServerAction(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseServerAction",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_create(
        &self,
        req: DatabaseCreateRequest,
    ) -> Result<DatabaseCreateResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-create-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseCreate(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseCreated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseCreated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_delete(
        &self,
        req: DatabaseDeleteRequest,
    ) -> Result<DatabaseDeleteResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-del-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseDelete(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseDeleted(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseDeleted",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_rotate_password(
        &self,
        req: DatabaseRotatePasswordRequest,
    ) -> Result<DatabaseRotatePasswordResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-rot-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseRotatePassword(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabasePasswordRotated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabasePasswordRotated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_update_remote_access(
        &self,
        req: DatabaseUpdateRemoteAccessRequest,
    ) -> Result<DatabaseUpdateRemoteAccessResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-rem-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseUpdateRemoteAccess(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseRemoteAccessUpdated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseRemoteAccessUpdated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_export(
        &self,
        req: DatabaseExportRequest,
    ) -> Result<DatabaseExportResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-exp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseExport(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseExported(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseExported",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_import(
        &self,
        req: DatabaseImportRequest,
    ) -> Result<DatabaseImportResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-imp-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseImport(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseImported(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseImported",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn database_size_check(
        &self,
        req: DatabaseSizeCheckRequest,
    ) -> Result<DatabaseSizeCheckResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "db-size-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::DatabaseSizeCheck(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::DatabaseSizeChecked(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "DatabaseSizeChecked",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn backup_create(
        &self,
        req: BackupCreateRequest,
    ) -> Result<BackupCreateResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "bk-create-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::BackupCreate(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::BackupCreated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "BackupCreated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn backup_restore(
        &self,
        req: BackupRestoreRequest,
    ) -> Result<BackupRestoreResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "bk-restore-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::BackupRestore(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::BackupRestored(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "BackupRestored",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn backup_delete(
        &self,
        req: BackupDeleteRequest,
    ) -> Result<BackupDeleteResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "bk-del-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::BackupDelete(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::BackupDeleted(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "BackupDeleted",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn optimizer_status(&self) -> Result<OptimizerStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "optimizer-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(
            req_id,
            RequestPayload::OptimizerGetStatus(OptimizerGetStatusRequest {}),
        );
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::OptimizerStatus(data) => Ok(*data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "OptimizerStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn optimizer_apply(
        &self,
        req: OptimizerApplyRequest,
    ) -> Result<OptimizerApplyResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "optimizer-apply-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::OptimizerApply(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::OptimizerApplied(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "OptimizerApplied",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn optimizer_rollback(
        &self,
        req: OptimizerRollbackRequest,
    ) -> Result<OptimizerRollbackResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "optimizer-rollback-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::OptimizerRollback(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::OptimizerRollbacked(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "OptimizerRollbacked",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn nftables_sync(
        &self,
        req: NftablesSyncRequest,
    ) -> Result<NftablesSyncResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "nftables-sync-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NftablesSync(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NftablesSynced(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NftablesSynced",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn nftables_status(
        &self,
        req: NftablesStatusRequest,
    ) -> Result<NftablesStatusResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "nftables-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::NftablesStatus(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::NftablesStatus(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "NftablesStatus",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn system_doctor_check(
        &self,
        req: SystemDoctorCheckRequest,
    ) -> Result<SystemDoctorCheckResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "doctor-check-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SystemDoctorCheck(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SystemDoctorChecked(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SystemDoctorChecked",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn system_doctor_fix(
        &self,
        req: SystemDoctorFixRequest,
    ) -> Result<SystemDoctorFixResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "doctor-fix-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SystemDoctorFix(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SystemDoctorFixed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SystemDoctorFixed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn system_reconcile(
        &self,
        req: SystemReconcileRequest,
    ) -> Result<SystemReconcileResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "reconcile-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SystemReconcile(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SystemReconciled(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SystemReconciled",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn system_backup_create(
        &self,
        req: SystemBackupCreateRequest,
    ) -> Result<SystemBackupCreateResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "backup-create-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SystemBackupCreate(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SystemBackupCreated(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SystemBackupCreated",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn system_backup_list(
        &self,
        req: SystemBackupListRequest,
    ) -> Result<SystemBackupListResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "backup-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SystemBackupList(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SystemBackupListed(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SystemBackupListed",
                got: format!("{other:?}"),
            }),
        }
    }

    pub async fn system_backup_restore(
        &self,
        req: SystemBackupRestoreRequest,
    ) -> Result<SystemBackupRestoreResponse, AgentClientError> {
        let mut rpc = self.connect().await?;
        let req_id = format!(
            "backup-restore-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );
        let rpc_req = RpcRequest::new(req_id, RequestPayload::SystemBackupRestore(req));
        rpc.send_request(&rpc_req).await?;
        let res = rpc.recv_response().await?;
        if !res.success {
            return Err(AgentClientError::AgentError(
                res.error
                    .unwrap_or_else(|| "Unknown agent error".to_string()),
            ));
        }
        match res.payload {
            ResponsePayload::SystemBackupRestored(data) => Ok(data),
            other => Err(AgentClientError::UnexpectedResponse {
                expected: "SystemBackupRestored",
                got: format!("{other:?}"),
            }),
        }
    }
}

pub fn plan_to_quota_set_request(
    uid: u32,
    username: impl Into<String>,
    plan: &panel_db::models::Plan,
) -> QuotaSetRequest {
    QuotaSetRequest {
        uid,
        username: username.into(),
        disk_limit_bytes: plan.disk_limit_bytes as u64,
        inode_limit: plan.inode_limit as u64,
        mount_point: None,
    }
}

pub fn plan_to_slice_apply_request(uid: u32, plan: &panel_db::models::Plan) -> SliceApplyRequest {
    SliceApplyRequest {
        uid,
        memory_max: Some(format!("{}M", plan.memory_limit_bytes / (1024 * 1024))),
        memory_high: plan
            .memory_high_bytes
            .map(|b| format!("{}M", b / (1024 * 1024))),
        cpu_quota: Some(format!("{}%", plan.cpu_quota_percent)),
        tasks_max: Some(plan.tasks_max as u64),
        io_weight: Some(plan.io_weight as u32),
    }
}
