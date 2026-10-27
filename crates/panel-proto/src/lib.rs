//! `panel-proto`: RPC protocol types, versioning, and length-delimited framing.

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

mod ai;
mod storage;
pub use storage::*;
mod connectivity;
pub use ai::*;
mod dns01;
mod panel_tls;
mod panel_update;
mod server_backup;
pub use connectivity::*;
pub use dns01::*;
pub use panel_tls::*;
pub use panel_update::*;
pub use server_backup::*;
mod cloudflare;
pub use cloudflare::*;
mod email;
pub use email::*;
mod database_web;
pub use database_web::*;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", content = "request")]
pub enum InstallOperation {
    EmailServer(EmailServerConfig),
    CloudflareConnector,
    PhpRepository,
    PhpRuntime(PhpRuntimeInstallRequest),
    DatabaseServer(DatabaseServerInstallRequest),
    DatabaseWebTools,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationProgress {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Protocol version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },
    #[error("Connection closed prematurely")]
    ConnectionClosed,
    #[error("Agent error: {0}")]
    AgentError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcRequest {
    pub proto_version: u32,
    pub request_id: String,
    pub payload: RequestPayload,
}

impl RpcRequest {
    pub fn new(request_id: impl Into<String>, payload: RequestPayload) -> Self {
        Self {
            proto_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd", content = "data")]
pub enum RequestPayload {
    AiSecret(AiSecretRequest),
    StorageSecret(StorageSecretRequest),
    ManagedStorage(ManagedStorageRequest),
    PanelUpdate(PanelUpdateRequest),
    ServerBackup(ServerBackupRequest),
    /// Read-only inspection of the fixed local OpenSSH host identity.
    SshHostKeys,
    ConnectivityInspect(ConnectivityTarget),
    PanelTls(PanelTlsRequest),
    Dns01(Dns01Request),
    Email(EmailRequest),
    Cloudflare(CloudflareRequest),
    /// Opt-in streaming for a closed set of package-manager operations.
    InstallWithProgress(InstallOperation),
    Ping,
    Version,
    UserCreate(UserCreateRequest),
    UserSuspend(UserSuspendRequest),
    UserUnsuspend(UserUnsuspendRequest),
    UserDelete(UserDeleteRequest),
    SliceApply(SliceApplyRequest),
    SliceRemove(SliceRemoveRequest),
    QuotaSet(QuotaSetRequest),
    QuotaGet(QuotaGetRequest),
    QuotaScan(QuotaScanRequest),
    ResourceRead(ResourceReadRequest),
    HardeningCheck,
    HardeningApply(HardeningApplyRequest),
    NginxBaselineApply(NginxBaselineApplyRequest),
    SiteApply(SiteApplyRequest),
    SiteRemove(SiteRemoveRequest),
    SiteHtpasswdDeploy(SiteHtpasswdDeployRequest),
    SiteHtpasswdRemove(SiteHtpasswdRemoveRequest),
    SiteProvision(SiteProvisionRequest),
    AcmeChallengeDeploy(AcmeChallengeDeployRequest),
    AcmeChallengeCleanup(AcmeChallengeCleanupRequest),
    SslCertDeploy(SslCertDeployRequest),
    SiteLogsRead(SiteLogsReadRequest),
    PhpRepoSetup(PhpRepoSetupRequest),
    PhpRuntimeList(PhpRuntimeListRequest),
    PhpRuntimeInstall(PhpRuntimeInstallRequest),
    PhpRuntimeRemove(PhpRuntimeRemoveRequest),
    PhpExtensionList(PhpExtensionListRequest),
    PhpExtensionInstall(PhpExtensionInstallRequest),
    PhpExtensionToggle(PhpExtensionToggleRequest),
    PhpExtensionRemove(PhpExtensionRemoveRequest),
    PhpFpmMasterEnsure(PhpFpmMasterEnsureRequest),
    PhpFpmPoolApply(PhpFpmPoolApplyRequest),
    PhpFpmPoolRemove(PhpFpmPoolRemoveRequest),
    PhpFpmMasterStatus(PhpFpmMasterStatusRequest),
    PhpFpmMasterReload(PhpFpmMasterReloadRequest),
    PhpFpmMasterRestart(PhpFpmMasterRestartRequest),
    PhpFpmHealthCheck(PhpFpmHealthCheckRequest),
    PhpFpmPoolStatus(PhpFpmPoolStatusRequest),
    PhpLogsRead(PhpLogsReadRequest),
    FsList(FsListRequest),
    FsStat(FsStatRequest),
    FsMkdir(FsMkdirRequest),
    FsRename(FsRenameRequest),
    FsCopy(FsCopyRequest),
    FsTrash(FsTrashRequest),
    FsTrashList(FsTrashListRequest),
    FsTrashRestore(FsTrashRestoreRequest),
    FsTrashEmpty(FsTrashEmptyRequest),
    FsDelete(FsDeleteRequest),
    FsUploadInit(FsUploadInitRequest),
    FsUploadGetSession(FsUploadGetSessionRequest),
    FsUploadAppendChunk(FsUploadAppendChunkRequest),
    FsReadRange(FsReadRangeRequest),
    FsZipDirectory(FsZipDirectoryRequest),
    FsArchiveCompress(FsArchiveCompressRequest),
    FsArchiveExtract(FsArchiveExtractRequest),
    FsChmod(FsChmodRequest),
    FsChmodRecursive(FsChmodRecursiveRequest),
    FsChown(FsChownRequest),
    FsReadFile(FsReadFileRequest),
    FsWriteAtomic(FsWriteAtomicRequest),
    FsWriteEditor(FsWriteEditorRequest),
    FsSearch(FsSearchRequest),
    PtyOpen(PtyOpenRequest),
    PtyInput(PtyInputRequest),
    PtyReadOutput(PtyReadOutputRequest),
    PtyResize(PtyResizeRequest),
    PtyClose(PtyCloseRequest),
    PtyList(PtyListRequest),
    SshKeysSync(SshKeysSyncRequest),
    SshPolicyApply(SshPolicyApplyRequest),
    SshDropinApply(SshDropinApplyRequest),
    NodeRuntimeList(NodeRuntimeListRequest),
    NodeRuntimeInstall(NodeRuntimeInstallRequest),
    NodeRuntimeRemove(NodeRuntimeRemoveRequest),
    NodeRuntimeCorepack(NodeRuntimeCorepackRequest),
    NodeSiteApply(NodeSiteApplyRequest),
    NodeSiteRemove(NodeSiteRemoveRequest),
    NodeSiteAction(NodeSiteActionRequest),
    NodeSiteStatus(NodeSiteStatusRequest),
    NodeSiteJob(NodeSiteJobRequest),
    PythonRuntimeList(PythonRuntimeListRequest),
    PythonRuntimeInstall(PythonRuntimeInstallRequest),
    PythonRuntimeRemove(PythonRuntimeRemoveRequest),
    PythonSiteApply(PythonSiteApplyRequest),
    PythonSiteRemove(PythonSiteRemoveRequest),
    PythonSiteAction(PythonSiteActionRequest),
    PythonSiteStatus(PythonSiteStatusRequest),
    PythonSiteJob(PythonSiteJobRequest),
    ProxyUpstreamProbe(ProxyUpstreamProbeRequest),
    SiteEnvEncrypt(SiteEnvEncryptRequest),
    SiteEnvDecrypt(SiteEnvDecryptRequest),
    SiteEnvApply(SiteEnvApplyRequest),
    CronJobApply(CronJobApplyRequest),
    CronJobRemove(CronJobRemoveRequest),
    CronJobRunNow(CronJobRunNowRequest),
    CronJobLogsRead(CronJobLogsReadRequest),
    SiteDeployKeyGenerate(SiteDeployKeyGenerateRequest),
    SiteGitDeploy(SiteGitDeployRequest),
    SiteGitRollback(SiteGitRollbackRequest),
    SiteGitReleasesList(SiteGitReleasesListRequest),
    DatabaseServerStatus(DatabaseServerStatusRequest),
    DatabaseServerInstall(DatabaseServerInstallRequest),
    DatabaseServerAction(DatabaseServerActionRequest),
    DatabaseCreate(DatabaseCreateRequest),
    DatabaseDelete(DatabaseDeleteRequest),
    DatabaseRotatePassword(DatabaseRotatePasswordRequest),
    DatabaseUpdateRemoteAccess(DatabaseUpdateRemoteAccessRequest),
    DatabaseExport(DatabaseExportRequest),
    DatabaseImport(DatabaseImportRequest),
    DatabaseSizeCheck(DatabaseSizeCheckRequest),
    DatabaseWebStatus,
    DatabaseWebCredentials(DatabaseWebCredentialsRequest),
    BackupCreate(BackupCreateRequest),
    BackupRestore(BackupRestoreRequest),
    BackupDelete(BackupDeleteRequest),
    OptimizerGetStatus(OptimizerGetStatusRequest),
    OptimizerApply(OptimizerApplyRequest),
    OptimizerRollback(OptimizerRollbackRequest),
    NftablesSync(NftablesSyncRequest),
    NftablesStatus(NftablesStatusRequest),
    SystemDoctorCheck(SystemDoctorCheckRequest),
    SystemDoctorFix(SystemDoctorFixRequest),
    SystemReconcile(SystemReconcileRequest),
    SystemBackupCreate(SystemBackupCreateRequest),
    SystemBackupList(SystemBackupListRequest),
    SystemBackupRestore(SystemBackupRestoreRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceApplyRequest {
    pub uid: u32,
    pub memory_max: Option<String>,
    pub memory_high: Option<String>,
    pub cpu_quota: Option<String>,
    pub tasks_max: Option<u64>,
    pub io_weight: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceRemoveRequest {
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaSetRequest {
    pub uid: u32,
    pub username: String,
    pub disk_limit_bytes: u64,
    pub inode_limit: u64,
    pub mount_point: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaGetRequest {
    pub uid: u32,
    pub username: String,
    pub mount_point: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaScanRequest {
    pub uid: u32,
    pub username: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceReadRequest {
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserCreateRequest {
    pub username: String,
    pub uid: u32,
    pub shell: Option<String>,
    pub home_dir: Option<String>,
    pub web_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserSuspendRequest {
    pub username: String,
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserUnsuspendRequest {
    pub username: String,
    pub uid: u32,
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserDeleteRequest {
    pub username: String,
    pub uid: u32,
    pub purge: bool,
    pub trash_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcResponse {
    pub proto_version: u32,
    pub request_id: String,
    pub success: bool,
    pub payload: ResponsePayload,
    pub error: Option<String>,
}

impl RpcResponse {
    pub fn ok(request_id: impl Into<String>, payload: ResponsePayload) -> Self {
        Self {
            proto_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            success: true,
            payload,
            error: None,
        }
    }

    pub fn err(request_id: impl Into<String>, message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            proto_version: PROTOCOL_VERSION,
            request_id: request_id.into(),
            success: false,
            payload: ResponsePayload::Error(msg.clone()),
            error: Some(msg),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ResponsePayload {
    AiSecret(AiSecret),
    StorageSecret(StorageSecret),
    ManagedStorage(serde_json::Value),
    PanelUpdate(PanelUpdateStatus),
    ServerBackup(serde_json::Value),
    SshHostKeys(Vec<panel_core::ssh::SshHostKeyFingerprint>),
    ConnectivityInspected(ConnectivityInspection),
    PanelTls(PanelTlsStatus),
    Dns01(serde_json::Value),
    Email(serde_json::Value),
    Cloudflare(serde_json::Value),
    InstallationProgress(InstallationProgress),
    Pong,
    Version(VersionInfo),
    UserCreated(UserCreatedResponse),
    UserSuspended(UserSuspendedResponse),
    UserUnsuspended(UserUnsuspendedResponse),
    UserDeleted(UserDeletedResponse),
    SliceApplied(SliceAppliedResponse),
    SliceRemoved(SliceRemovedResponse),
    QuotaSet(QuotaSetResponse),
    QuotaGet(QuotaGetResponse),
    QuotaScan(QuotaScanResponse),
    ResourceRead(ResourceReadResponse),
    HardeningStatus(Box<HardeningStatus>),
    HardeningApplied(HardeningApplyResponse),
    NginxBaselineApplied(NginxBaselineAppliedResponse),
    SiteApplied(SiteAppliedResponse),
    SiteRemoved(SiteRemovedResponse),
    SiteHtpasswdDeployed(SiteHtpasswdDeployedResponse),
    SiteHtpasswdRemoved(SiteHtpasswdRemovedResponse),
    SiteProvisioned(SiteProvisionResponse),
    AcmeChallengeDeployed(AcmeChallengeDeployedResponse),
    AcmeChallengeCleaned(AcmeChallengeCleanedResponse),
    SslCertDeployed(SslCertDeployedResponse),
    SiteLogsRead(SiteLogsReadResponse),
    PhpRepoSetup(PhpRepoSetupResponse),
    PhpRuntimeList(PhpRuntimeListResponse),
    PhpRuntimeInstalled(PhpRuntimeActionResponse),
    PhpRuntimeRemoved(PhpRuntimeActionResponse),
    PhpExtensionList(PhpExtensionListResponse),
    PhpExtensionAction(PhpExtensionActionResponse),
    PhpFpmMasterEnsure(PhpFpmMasterEnsureResponse),
    PhpFpmPoolApply(PhpFpmPoolApplyResponse),
    PhpFpmPoolRemove(PhpFpmPoolRemoveResponse),
    PhpFpmMasterStatus(PhpFpmMasterStatusResponse),
    PhpFpmMasterAction(PhpFpmMasterActionResponse),
    PhpFpmHealthChecked(PhpFpmHealthCheckResponse),
    PhpFpmPoolStatus(PhpFpmPoolStatusResponse),
    PhpLogsRead(PhpLogsReadResponse),
    FsList(FsListResponse),
    FsStat(FsStatResponse),
    FsTrash(FsTrashResponse),
    FsTrashList(FsTrashListResponse),
    FsTrashRestore(FsTrashRestoreResponse),
    FsTrashEmpty(FsTrashEmptyResponse),
    FsAction(FsActionResponse),
    FsUploadInit(FsUploadInitResponse),
    FsUploadGetSession(FsUploadGetSessionResponse),
    FsUploadAppendChunk(FsUploadAppendChunkResponse),
    FsReadRange(FsReadRangeResponse),
    FsZipDirectory(FsZipDirectoryResponse),
    FsArchiveCompress(FsArchiveCompressResponse),
    FsArchiveExtract(FsArchiveExtractResponse),
    FsChmod(FsChmodResponse),
    FsChmodRecursive(FsChmodRecursiveResponse),
    FsChown(FsChownResponse),
    FsReadFile(FsReadFileResponse),
    FsWriteAtomic(FsWriteAtomicResponse),
    FsWriteEditor(FsWriteAtomicResponse),
    FsSearch(FsSearchResponse),
    PtyOpen(PtyOpenResponse),
    PtyInput(PtyInputResponse),
    PtyReadOutput(PtyReadOutputResponse),
    PtyResize(PtyResizeResponse),
    PtyClose(PtyCloseResponse),
    PtyList(PtyListResponse),
    SshKeysSync(SshKeysSyncResponse),
    SshPolicyApply(SshPolicyApplyResponse),
    SshDropinApply(SshDropinApplyResponse),
    NodeRuntimeList(NodeRuntimeListResponse),
    NodeRuntimeInstalled(NodeRuntimeActionResponse),
    NodeRuntimeRemoved(NodeRuntimeActionResponse),
    NodeRuntimeCorepack(NodeRuntimeActionResponse),
    NodeSiteApplied(NodeSiteApplyResponse),
    NodeSiteRemoved(NodeSiteActionResponse),
    NodeSiteAction(NodeSiteActionResponse),
    NodeSiteStatus(NodeSiteStatusResponse),
    NodeSiteJob(NodeSiteJobResponse),
    PythonRuntimeList(PythonRuntimeListResponse),
    PythonRuntimeInstalled(PythonRuntimeActionResponse),
    PythonRuntimeRemoved(PythonRuntimeActionResponse),
    PythonSiteApplied(PythonSiteApplyResponse),
    PythonSiteRemoved(PythonSiteActionResponse),
    PythonSiteAction(PythonSiteActionResponse),
    PythonSiteStatus(PythonSiteStatusResponse),
    PythonSiteJob(PythonSiteJobResponse),
    ProxyUpstreamProbed(ProxyUpstreamProbeResponse),
    SiteEnvEncrypted(SiteEnvEncryptResponse),
    SiteEnvDecrypted(SiteEnvDecryptResponse),
    SiteEnvApplied(SiteEnvApplyResponse),
    CronJobApplied(CronJobApplyResponse),
    CronJobRemoved(CronJobRemoveResponse),
    CronJobRunNow(CronJobRunNowResponse),
    CronJobLogsRead(CronJobLogsReadResponse),
    SiteDeployKeyGenerated(SiteDeployKeyGenerateResponse),
    SiteGitDeployed(SiteGitDeployResponse),
    SiteGitRollbacked(SiteGitRollbackResponse),
    SiteGitReleasesList(SiteGitReleasesListResponse),
    DatabaseServerStatus(DatabaseServerStatusResponse),
    DatabaseServerInstalled(DatabaseServerInstallResponse),
    DatabaseServerAction(DatabaseServerActionResponse),
    DatabaseCreated(DatabaseCreateResponse),
    DatabaseDeleted(DatabaseDeleteResponse),
    DatabasePasswordRotated(DatabaseRotatePasswordResponse),
    DatabaseRemoteAccessUpdated(DatabaseUpdateRemoteAccessResponse),
    DatabaseExported(DatabaseExportResponse),
    DatabaseImported(DatabaseImportResponse),
    DatabaseSizeChecked(DatabaseSizeCheckResponse),
    DatabaseWebStatus(DatabaseWebStatus),
    DatabaseWebCredentials(DatabaseWebCredentials),
    BackupCreated(BackupCreateResponse),
    BackupRestored(BackupRestoreResponse),
    BackupDeleted(BackupDeleteResponse),
    OptimizerStatus(Box<OptimizerStatusResponse>),
    OptimizerApplied(OptimizerApplyResponse),
    OptimizerRollbacked(OptimizerRollbackResponse),
    NftablesSynced(NftablesSyncResponse),
    NftablesStatus(NftablesStatusResponse),
    SystemDoctorChecked(SystemDoctorCheckResponse),
    SystemDoctorFixed(SystemDoctorFixResponse),
    SystemReconciled(SystemReconcileResponse),
    SystemBackupCreated(SystemBackupCreateResponse),
    SystemBackupListed(SystemBackupListResponse),
    SystemBackupRestored(SystemBackupRestoreResponse),
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsEntryDto {
    pub name: String,
    pub relative_path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub created_at: Option<i64>,
    pub mode: u32,
    pub mode_octal: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsListRequest {
    pub username: String,
    pub path: String,
    pub limit: usize,
    pub offset: usize,
    pub sort_by: String,
    pub sort_order: String,
    pub show_hidden: bool,
    pub search_query: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsListResponse {
    pub entries: Vec<FsEntryDto>,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsStatRequest {
    pub username: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsStatResponse {
    pub entry: FsEntryDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsMkdirRequest {
    pub username: String,
    pub path: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsRenameRequest {
    pub username: String,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsCopyRequest {
    pub username: String,
    pub source: String,
    pub destination: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrashedItemDto {
    pub trash_id: String,
    pub original_path: String,
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub trashed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashRequest {
    pub username: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashResponse {
    pub item: TrashedItemDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashListRequest {
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashListResponse {
    pub items: Vec<TrashedItemDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashRestoreRequest {
    pub username: String,
    pub trash_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashRestoreResponse {
    pub item: TrashedItemDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashEmptyRequest {
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsTrashEmptyResponse {
    pub purged_count: usize,
    pub bytes_freed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsDeleteRequest {
    pub username: String,
    pub path: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadMetadataDto {
    pub upload_id: String,
    pub file_name: String,
    pub target_path: String,
    pub total_size: u64,
    pub current_offset: u64,
    pub created_at: i64,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsUploadInitRequest {
    pub username: String,
    pub target_path: String,
    pub file_name: String,
    pub total_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsUploadInitResponse {
    pub metadata: UploadMetadataDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsUploadGetSessionRequest {
    pub username: String,
    pub upload_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsUploadGetSessionResponse {
    pub metadata: UploadMetadataDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsUploadAppendChunkRequest {
    pub username: String,
    pub upload_id: String,
    pub offset: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsUploadAppendChunkResponse {
    pub metadata: UploadMetadataDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsReadRangeRequest {
    pub username: String,
    pub path: String,
    pub start: u64,
    pub length: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsReadRangeResponse {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsZipDirectoryRequest {
    pub username: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsZipDirectoryResponse {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormatDto {
    Zip,
    TarGz,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveResultDto {
    pub entries_processed: usize,
    pub total_bytes: u64,
    pub target_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsArchiveCompressRequest {
    pub username: String,
    pub source_paths: Vec<String>,
    pub destination: String,
    pub format: ArchiveFormatDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsArchiveCompressResponse {
    pub result: ArchiveResultDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsArchiveExtractRequest {
    pub username: String,
    pub archive_path: String,
    pub destination_dir: String,
    pub format: Option<ArchiveFormatDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsArchiveExtractResponse {
    pub result: ArchiveResultDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsChmodRequest {
    pub username: String,
    pub path: String,
    pub mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsChmodResponse {
    pub path: String,
    pub mode: u32,
    pub mode_octal: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsChmodRecursiveRequest {
    pub username: String,
    pub path: String,
    pub file_mode: u32,
    pub dir_mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsChmodRecursiveResponse {
    pub path: String,
    pub file_mode: u32,
    pub file_mode_octal: String,
    pub dir_mode: u32,
    pub dir_mode_octal: String,
    pub affected_count: usize,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsChownRequest {
    pub username: String,
    pub path: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub caller_uid: u32,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsChownResponse {
    pub path: String,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsReadFileRequest {
    pub username: String,
    pub path: String,
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsReadFileResponse {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsWriteAtomicRequest {
    pub username: String,
    pub path: String,
    pub data: Vec<u8>,
    pub mode: Option<u32>,
}

/// Dedicated editor operation: older agents reject the variant instead of ignoring a condition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsWriteEditorRequest {
    pub username: String,
    pub path: String,
    pub data: Vec<u8>,
    pub expected_etag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsWriteAtomicResponse {
    pub bytes_written: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContentMatchDto {
    pub line_number: usize,
    pub line_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsSearchItemDto {
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_at: Option<i64>,
    pub matches: Vec<ContentMatchDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsSearchRequest {
    pub username: String,
    pub path: String,
    pub query: String,
    pub search_content: bool,
    pub case_sensitive: bool,
    pub extension: Option<String>,
    pub max_results: Option<usize>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsSearchResponse {
    pub items: Vec<FsSearchItemDto>,
    pub total_scanned: usize,
    pub limit_reached: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyOpenRequest {
    pub username: String,
    pub uid: u32,
    pub gid: u32,
    pub cwd: Option<String>,
    pub rows: u16,
    pub cols: u16,
    pub term: Option<String>,
    pub max_sessions: Option<usize>,
    pub idle_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyOpenResponse {
    pub session_id: String,
    pub username: String,
    pub pid: u32,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyInputRequest {
    pub session_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyInputResponse {
    pub session_id: String,
    pub bytes_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyReadOutputRequest {
    pub session_id: String,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyReadOutputResponse {
    pub session_id: String,
    pub data: Vec<u8>,
    pub closed: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyResizeRequest {
    pub session_id: String,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyResizeResponse {
    pub session_id: String,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyCloseRequest {
    pub session_id: String,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyCloseResponse {
    pub session_id: String,
    pub closed: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyListRequest {
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtySessionInfoDto {
    pub session_id: String,
    pub username: String,
    pub uid: u32,
    pub pid: u32,
    pub cwd: String,
    pub rows: u16,
    pub cols: u16,
    pub created_at: i64,
    pub last_active_at: i64,
    pub is_alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PtyListResponse {
    pub sessions: Vec<PtySessionInfoDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserCreatedResponse {
    pub username: String,
    pub uid: u32,
    pub home_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserSuspendedResponse {
    pub username: String,
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserUnsuspendedResponse {
    pub username: String,
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserDeletedResponse {
    pub username: String,
    pub uid: u32,
    pub purged: bool,
    pub archived_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceAppliedResponse {
    pub uid: u32,
    pub slice_name: String,
    pub dropin_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SliceRemovedResponse {
    pub uid: u32,
    pub slice_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaSetResponse {
    pub uid: u32,
    pub username: String,
    pub applied: bool,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaGetResponse {
    pub uid: u32,
    pub username: String,
    pub bytes_used: u64,
    pub bytes_limit: u64,
    pub inodes_used: u64,
    pub inodes_limit: u64,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuotaScanResponse {
    pub uid: u32,
    pub username: String,
    pub bytes_used: u64,
    pub inodes_used: u64,
    pub scan_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SliceResourceMetrics {
    pub uid: u32,
    pub slice_name: String,
    pub exists: bool,
    pub cpu_usage_usec: u64,
    pub cpu_user_usec: u64,
    pub cpu_system_usec: u64,
    pub cpu_nr_periods: u64,
    pub cpu_nr_throttled: u64,
    pub cpu_throttled_usec: u64,
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub pids_current: u64,
    pub pids_peak: Option<u64>,
    pub io_read_bytes: u64,
    pub io_write_bytes: u64,
    pub io_read_ops: u64,
    pub io_write_ops: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceReadResponse {
    pub metrics: SliceResourceMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrerequisiteCheckItem {
    pub name: String,
    pub ok: bool,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardeningStatus {
    pub cgroup_v2: PrerequisiteCheckItem,
    pub acl: PrerequisiteCheckItem,
    pub quota: PrerequisiteCheckItem,
    pub hidepid: PrerequisiteCheckItem,
    pub tenant_group: PrerequisiteCheckItem,
    pub overall_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardeningApplyRequest {
    pub enable_hidepid: bool,
    pub ensure_tenant_group: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardeningApplyResponse {
    pub hidepid_applied: bool,
    pub tenant_group_created: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NginxBaselineApplyRequest {
    pub base_dir: Option<String>,
    pub log_dir: Option<String>,
    pub acme_dir: Option<String>,
    pub panel_api_port: Option<u16>,
    pub panel_listen_port: Option<u16>,
    pub test_config: bool,
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NginxBaselineAppliedResponse {
    pub base_dir: String,
    pub nginx_conf_path: String,
    pub conf_d_dir: String,
    pub snippets_dir: String,
    pub sites_available_dir: String,
    pub sites_enabled_dir: String,
    pub log_dir: String,
    pub acme_dir: String,
    pub test_passed: bool,
    pub reloaded: bool,
}

/// Validated again by the agent; log paths are derived internally, never supplied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SiteLogContext {
    pub username: String,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteApplyRequest {
    pub site_id: String,
    pub config_content: String,
    pub enable: bool,
    pub nginx_base_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_context: Option<SiteLogContext>,
    pub test_config: bool,
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteAppliedResponse {
    pub site_id: String,
    pub available_path: String,
    pub enabled_path: Option<String>,
    pub enabled: bool,
    pub reloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteRemoveRequest {
    pub site_id: String,
    pub nginx_base_dir: Option<String>,
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteRemovedResponse {
    pub site_id: String,
    pub reloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteHtpasswdDeployRequest {
    pub site_id: String,
    pub htpasswd_content: String,
    pub auth_base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteHtpasswdDeployedResponse {
    pub site_id: String,
    pub htpasswd_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteHtpasswdRemoveRequest {
    pub site_id: String,
    pub auth_base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteHtpasswdRemovedResponse {
    pub site_id: String,
    pub removed: bool,
}

/// Fixed, agent-owned starter destinations; no free-form file path is accepted.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteStarter {
    #[default]
    Html,
    Php,
    Nodejs,
    PythonApp,
    PythonMain,
    PythonWsgi,
    PythonDjango,
    Empty,
}

impl SiteStarter {
    pub fn filename(self) -> Option<&'static str> {
        match self {
            Self::Html => Some("index.html"),
            Self::Php => Some("index.php"),
            Self::Nodejs => Some("server.js"),
            Self::PythonApp => Some("app.py"),
            Self::PythonMain => Some("main.py"),
            Self::PythonWsgi => Some("wsgi.py"),
            Self::PythonDjango => Some("myproject/wsgi.py"),
            Self::Empty => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteProvisionRequest {
    pub site_id: i64,
    pub username: String,
    pub uid: u32,
    pub home_dir: String,
    pub docroot: String,
    pub web_user: Option<String>,
    pub index_content: Option<String>,
    #[serde(default)]
    pub starter: SiteStarter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteProvisionResponse {
    pub docroot: String,
    pub placeholder_written: bool,
    pub acls_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcmeChallengeDeployRequest {
    pub token: String,
    pub key_authorization: String,
    pub acme_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcmeChallengeDeployedResponse {
    pub challenge_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcmeChallengeCleanupRequest {
    pub token: String,
    pub acme_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcmeChallengeCleanedResponse {
    pub token: String,
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SslCertDeployRequest {
    pub domain: String,
    pub fullchain_pem: String,
    pub privkey_pem: String,
    pub ssl_base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SslCertDeployedResponse {
    pub domain: String,
    pub cert_path: String,
    pub key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteLogsReadRequest {
    pub username: String,
    pub site_domain: String,
    pub log_type: String,
    pub lines: usize,
    pub query: Option<String>,
    pub log_base_dir: Option<String>,
    #[serde(default)]
    pub site_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteLogsReadResponse {
    pub lines: Vec<String>,
    pub total_matched: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRepoSetupRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRepoSetupResponse {
    pub success: bool,
    pub ppa_configured: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRuntimeListRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpVersionStatusDto {
    pub version: String,
    pub installed: bool,
    pub fpm_binary_exists: bool,
    pub cli_binary_exists: bool,
    pub service_masked: bool,
    pub status: String,
    pub fpm_binary_path: String,
    pub cli_binary_path: String,
    pub service_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRuntimeListResponse {
    pub ppa_configured: bool,
    pub versions: Vec<PhpVersionStatusDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRuntimeInstallRequest {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRuntimeRemoveRequest {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpRuntimeActionResponse {
    pub version: String,
    pub success: bool,
    pub masked: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionDto {
    pub name: String,
    pub version: String,
    pub package_name: String,
    pub installed: bool,
    pub enabled: bool,
    pub is_base: bool,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionListRequest {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionListResponse {
    pub version: String,
    pub extensions: Vec<PhpExtensionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionInstallRequest {
    pub version: String,
    pub extension: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionToggleRequest {
    pub version: String,
    pub extension: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionRemoveRequest {
    pub version: String,
    pub extension: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpExtensionActionResponse {
    pub version: String,
    pub extension: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterEnsureRequest {
    pub username: String,
    pub uid: u32,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterEnsureResponse {
    pub service_name: String,
    pub config_dir: String,
    pub default_socket_path: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmPoolApplyRequest {
    pub username: String,
    pub uid: u32,
    pub version: String,
    pub pool_name: String,
    pub docroot: String,
    pub socket_path: Option<String>,
    pub max_children: Option<u32>,
    pub process_idle_timeout_secs: Option<u32>,
    pub max_requests: Option<u32>,
    pub open_basedir: Option<Vec<String>>,
    pub disable_functions: Option<Vec<String>>,
    #[serde(default)]
    pub settings: Option<panel_core::php::PhpSettings>,
    #[serde(default)]
    pub admin_ini: Option<String>,
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmPoolApplyResponse {
    pub pool_name: String,
    pub pool_conf_path: String,
    pub socket_path: String,
    pub reloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmPoolRemoveRequest {
    pub username: String,
    pub version: String,
    pub pool_name: String,
    pub reload: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmPoolRemoveResponse {
    pub pool_name: String,
    pub removed: bool,
    pub remaining_pools: usize,
    pub master_stopped: bool,
    pub reloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterStatusRequest {
    pub username: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PhpFpmStatusMetrics {
    pub pool: String,
    #[serde(rename = "process manager", default)]
    pub process_manager: String,
    #[serde(rename = "start time", default)]
    pub start_time: u64,
    #[serde(rename = "start since", default)]
    pub start_since: u64,
    #[serde(rename = "accepted conn", default)]
    pub accepted_conn: u64,
    #[serde(rename = "listen queue", default)]
    pub listen_queue: u32,
    #[serde(rename = "max listen queue", default)]
    pub max_listen_queue: u32,
    #[serde(rename = "listen queue len", default)]
    pub listen_queue_len: u32,
    #[serde(rename = "idle processes", default)]
    pub idle_processes: u32,
    #[serde(rename = "active processes", default)]
    pub active_processes: u32,
    #[serde(rename = "total processes", default)]
    pub total_processes: u32,
    #[serde(rename = "max active processes", default)]
    pub max_active_processes: u32,
    #[serde(rename = "max children reached", default)]
    pub max_children_reached: u32,
    #[serde(rename = "slow requests", default)]
    pub slow_requests: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterStatusResponse {
    pub service_name: String,
    pub is_active: bool,
    pub substate: String,
    pub socket_exists: bool,
    pub pools_count: usize,
    pub metrics: Option<PhpFpmStatusMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmPoolStatusRequest {
    pub username: String,
    pub version: String,
    pub pool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmPoolStatusResponse {
    pub pool_name: String,
    pub service_name: String,
    pub is_active: bool,
    pub socket_exists: bool,
    pub metrics: Option<PhpFpmStatusMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpLogsReadRequest {
    pub username: String,
    pub version: String,
    pub pool_name: Option<String>,
    pub log_type: String, // "error" or "slow"
    pub lines: usize,
    pub query: Option<String>,
    pub log_base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpLogsReadResponse {
    pub lines: Vec<String>,
    pub total_matched: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterReloadRequest {
    pub username: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterRestartRequest {
    pub username: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmMasterActionResponse {
    pub service_name: String,
    pub action: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmHealthCheckRequest {
    pub username: String,
    pub version: String,
    pub pool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmHealthCheckResponse {
    pub healthy: bool,
    pub service_active: bool,
    pub socket_exists: bool,
    pub pool_exists: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshKeysSyncRequest {
    pub linux_username: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshKeysSyncResponse {
    pub success: bool,
    pub path: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshPolicyApplyRequest {
    pub linux_username: String,
    pub uid: u32,
    pub policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshPolicyApplyResponse {
    pub success: bool,
    pub shell: String,
    pub sftp_group_member: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SshDropinApplyRequest {
    pub custom_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshDropinApplyResponse {
    pub success: bool,
    pub path: String,
    pub validated: bool,
    pub reloaded: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeInstalledVersionDto {
    pub version: String,
    pub raw_version: String,
    pub path: String,
    pub node_binary: String,
    pub npm_binary: Option<String>,
    pub npx_binary: Option<String>,
    pub corepack_binary: Option<String>,
    pub pnpm_binary: Option<String>,
    pub yarn_binary: Option<String>,
    pub corepack_enabled: bool,
    pub is_lts: bool,
    pub lts_codename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeListRequest {
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeListResponse {
    pub base_dir: String,
    pub versions: Vec<NodeInstalledVersionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeInstallRequest {
    pub version: String,
    pub mirror_url: Option<String>,
    pub offline_tarball_path: Option<String>,
    pub offline_checksum: Option<String>,
    pub enable_corepack: bool,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeRemoveRequest {
    pub version: String,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeCorepackRequest {
    pub version: String,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeRuntimeActionResponse {
    pub version: String,
    pub success: bool,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum NodeSiteEntrypoint {
    NodeScript(String),
    NpmStart,
    NpmCustom(String),
}

impl Default for NodeSiteEntrypoint {
    fn default() -> Self {
        Self::NodeScript("server.js".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteApplyRequest {
    pub site_id: String,
    pub username: String,
    pub uid: u32,
    pub node_version: String,
    pub app_dir: String,
    pub port: u16,
    pub entrypoint: NodeSiteEntrypoint,
    pub env_vars: std::collections::BTreeMap<String, String>,
    pub auto_restart: bool,
    pub env_file_path: Option<String>,
    pub unit_path: Option<String>,
    pub base_runtime_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteApplyResponse {
    pub site_id: String,
    pub service_name: String,
    pub unit_path: String,
    pub env_path: String,
    pub port: u16,
    pub active: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteRemoveRequest {
    pub site_id: String,
    pub unit_path: Option<String>,
    pub env_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeSiteActionType {
    Start,
    Stop,
    Restart,
    Reload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteActionRequest {
    pub site_id: String,
    pub action: NodeSiteActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteActionResponse {
    pub site_id: String,
    pub action: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteStatusRequest {
    pub site_id: String,
    pub port: Option<u16>,
    pub probe_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteStatusResponse {
    pub site_id: String,
    pub service_name: String,
    pub active: bool,
    pub sub_state: String,
    pub pid: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub healthy: bool,
    pub probe_status_code: Option<u16>,
    pub probe_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeSiteJobType {
    NpmInstall,
    NpmCi,
    NpmBuild,
    PnpmInstall,
    YarnInstall,
    CustomScript { script_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteJobRequest {
    pub site_id: String,
    pub username: String,
    pub uid: u32,
    pub node_version: String,
    pub app_dir: String,
    pub job_type: NodeSiteJobType,
    pub timeout_secs: Option<u64>,
    pub base_runtime_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSiteJobResponse {
    pub site_id: String,
    pub command: String,
    pub exit_code: i32,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonInstalledVersionDto {
    pub version: String,
    pub install_path: String,
    pub bin_path: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonRuntimeListRequest {
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonRuntimeListResponse {
    pub installed: Vec<PythonInstalledVersionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonRuntimeInstallRequest {
    pub version: String,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonRuntimeRemoveRequest {
    pub version: String,
    pub base_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonRuntimeActionResponse {
    pub version: String,
    pub success: bool,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PythonSiteFramework {
    Django,
    Flask,
    FastApi,
    #[default]
    Generic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PythonSiteServer {
    #[default]
    GunicornWsgi,
    UvicornAsgi,
    GunicornUvicornWorker,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PythonSiteEntrypoint {
    Wsgi(String),
    Asgi(String),
    CustomCommand(String),
}

impl Default for PythonSiteEntrypoint {
    fn default() -> Self {
        Self::Wsgi("app:app".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteApplyRequest {
    pub site_id: String,
    pub username: String,
    pub uid: u32,
    pub python_version: String,
    pub app_dir: String,
    pub venv_dir: String,
    pub socket_path: String,
    pub framework: PythonSiteFramework,
    pub server_type: PythonSiteServer,
    pub entrypoint: PythonSiteEntrypoint,
    pub workers: u32,
    pub env_vars: std::collections::BTreeMap<String, String>,
    pub auto_restart: bool,
    pub env_file_path: Option<String>,
    pub unit_path: Option<String>,
    pub base_runtime_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteApplyResponse {
    pub site_id: String,
    pub service_name: String,
    pub unit_path: String,
    pub env_path: String,
    pub socket_path: String,
    pub active: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteRemoveRequest {
    pub site_id: String,
    pub socket_path: Option<String>,
    pub unit_path: Option<String>,
    pub env_file_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PythonSiteActionType {
    Start,
    Stop,
    Restart,
    Redeploy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteActionRequest {
    pub site_id: String,
    pub action: PythonSiteActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteActionResponse {
    pub site_id: String,
    pub action: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteStatusRequest {
    pub site_id: String,
    pub socket_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteStatusResponse {
    pub site_id: String,
    pub service_name: String,
    pub active: bool,
    pub sub_state: String,
    pub pid: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage_usec: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub socket_exists: bool,
    pub healthy: bool,
    pub probe_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PythonSiteJobType {
    UvVenvCreate,
    UvPipInstallRequirements { req_file: Option<String> },
    UvPipInstallPyproject,
    CustomScript { script_name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteJobRequest {
    pub site_id: String,
    pub username: String,
    pub uid: u32,
    pub python_version: String,
    pub app_dir: String,
    pub venv_dir: String,
    pub job_type: PythonSiteJobType,
    pub timeout_secs: Option<u64>,
    pub base_runtime_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PythonSiteJobResponse {
    pub site_id: String,
    pub command: String,
    pub exit_code: i32,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyUpstreamProbeRequest {
    pub upstream_url: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyUpstreamProbeResponse {
    pub healthy: bool,
    pub status_code: Option<u16>,
    pub latency_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEnvEncryptRequest {
    pub site_id: String,
    pub env_vars: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEnvEncryptResponse {
    pub site_id: String,
    pub encrypted_vars: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEnvDecryptRequest {
    pub site_id: String,
    pub encrypted_vars: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEnvDecryptResponse {
    pub site_id: String,
    pub env_vars: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEnvApplyRequest {
    pub site_id: String,
    pub username: String,
    pub env_file_path: Option<String>,
    pub encrypted_vars: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteEnvApplyResponse {
    pub site_id: String,
    pub env_file_path: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobApplyRequest {
    pub site_id: String,
    pub job_id: i64,
    pub name: String,
    pub username: String,
    pub uid: u32,
    pub command: String,
    pub schedule: String,
    pub systemd_calendar: String,
    pub work_dir: Option<String>,
    pub timeout_secs: u64,
    pub enabled: bool,
    pub unit_dir: Option<String>,
    pub env_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobApplyResponse {
    pub site_id: String,
    pub job_id: i64,
    pub service_name: String,
    pub timer_name: String,
    pub enabled: bool,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobRemoveRequest {
    pub site_id: String,
    pub job_id: i64,
    pub unit_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobRemoveResponse {
    pub site_id: String,
    pub job_id: i64,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobRunNowRequest {
    pub site_id: String,
    pub job_id: i64,
    pub username: String,
    pub uid: u32,
    pub command: String,
    pub work_dir: Option<String>,
    pub timeout_secs: u64,
    pub env_file_path: Option<String>,
    pub unit_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobRunNowResponse {
    pub site_id: String,
    pub job_id: i64,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobLogsReadRequest {
    pub site_id: String,
    pub job_id: i64,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CronJobLogsReadResponse {
    pub site_id: String,
    pub job_id: i64,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteDeployKeyGenerateRequest {
    pub site_id: String,
    pub username: String,
    pub key_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteDeployKeyGenerateResponse {
    pub site_id: String,
    pub public_key: String,
    pub private_key_path: String,
    pub public_key_path: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteGitDeployRequest {
    pub site_id: String,
    pub username: String,
    pub uid: u32,
    pub repo_url: String,
    pub branch: String,
    pub commit_hash: Option<String>,
    pub base_dir: String,
    pub docroot_subpath: Option<String>,
    pub build_cmd: Option<String>,
    pub post_deploy_cmd: Option<String>,
    pub release_id: String,
    pub keep_releases: u32,
    pub deploy_key_path: Option<String>,
    pub env_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteGitDeployResponse {
    pub site_id: String,
    pub release_id: String,
    pub commit_hash: String,
    pub commit_author: String,
    pub commit_message: String,
    pub duration_ms: u64,
    pub success: bool,
    pub log: String,
    pub current_symlink_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteGitRollbackRequest {
    pub site_id: String,
    pub username: String,
    pub uid: u32,
    pub base_dir: String,
    pub target_release_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteGitRollbackResponse {
    pub site_id: String,
    pub release_id: String,
    pub success: bool,
    pub log: String,
    pub current_symlink_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteGitReleasesListRequest {
    pub base_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteGitReleasesListResponse {
    pub current_release: Option<String>,
    pub releases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerStatusRequest {
    pub engines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerStatusItem {
    pub engine: String,
    pub is_installed: bool,
    pub is_running: bool,
    pub version: Option<String>,
    pub port: u16,
    pub socket_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerStatusResponse {
    pub servers: Vec<DatabaseServerStatusItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerInstallRequest {
    pub engine: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerInstallResponse {
    pub engine: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerActionRequest {
    pub engine: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseServerActionResponse {
    pub engine: String,
    pub action: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseCreateRequest {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub password: String,
    pub remote_access: bool,
    pub allowed_hosts: String,
    pub collation: String,
    pub character_set: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseCreateResponse {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseDeleteRequest {
    pub engine: String,
    pub name: String,
    pub db_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseDeleteResponse {
    pub engine: String,
    pub name: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseRotatePasswordRequest {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub new_password: String,
    pub allowed_hosts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseRotatePasswordResponse {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseUpdateRemoteAccessRequest {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub remote_access: bool,
    pub allowed_hosts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseUpdateRemoteAccessResponse {
    pub engine: String,
    pub name: String,
    pub db_user: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseExportRequest {
    pub engine: String,
    pub name: String,
    pub db_user: Option<String>,
    pub export_file_path: String,
    pub tenant_username: String,
    pub tenant_uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseExportResponse {
    pub engine: String,
    pub name: String,
    pub file_path: String,
    pub size_bytes: u64,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseImportRequest {
    pub engine: String,
    pub name: String,
    pub db_user: Option<String>,
    pub import_file_path: String,
    pub tenant_username: String,
    pub tenant_uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseImportResponse {
    pub engine: String,
    pub name: String,
    pub file_path: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseSizeCheckRequest {
    pub engine: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatabaseSizeCheckResponse {
    pub engine: String,
    pub name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupDatabaseInfo {
    pub engine: String,
    pub name: String,
    pub username: String,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupCreateRequest {
    pub site_id: i64,
    pub tenant_username: String,
    pub tenant_uid: u32,
    pub site_dir: String,
    pub backup_dir: String,
    pub filename: String,
    pub include_files: bool,
    pub include_databases: bool,
    pub databases: Vec<BackupDatabaseInfo>,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupCreateResponse {
    pub success: bool,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub filepath: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupRestoreRequest {
    pub site_id: i64,
    pub tenant_username: String,
    pub tenant_uid: u32,
    pub backup_filepath: String,
    pub target_site_dir: String,
    pub restore_files: bool,
    pub restore_databases: bool,
    pub databases: Vec<BackupDatabaseInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupRestoreResponse {
    pub success: bool,
    pub restored_files_count: u64,
    pub restored_databases_count: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupDeleteRequest {
    pub filepath: String,
    pub tenant_username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupDeleteResponse {
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizerProfileType {
    Micro,
    Standard,
    HighPerformance,
    DedicatedLarge,
    Custom,
}

impl OptimizerProfileType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Micro => "micro",
            Self::Standard => "standard",
            Self::HighPerformance => "high_performance",
            Self::DedicatedLarge => "dedicated_large",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerHardwareSpecs {
    pub vcpu_count: u32,
    pub ram_total_mb: u64,
    pub ram_free_mb: u64,
    pub swap_total_mb: u64,
    pub swap_free_mb: u64,
    pub kernel_version: String,
    pub os_distro: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NginxTuningConfig {
    pub worker_processes: String,
    pub worker_connections: u32,
    pub worker_rlimit_nofile: u32,
    pub multi_accept: bool,
    pub keepalive_timeout_secs: u32,
    pub client_body_buffer_size_kb: u32,
    pub fastcgi_buffer_size_kb: u32,
    pub fastcgi_buffers: String,
    pub open_file_cache_max: u32,
    pub open_file_cache_inactive_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysctlTuningConfig {
    pub somaxconn: u32,
    pub tcp_max_syn_backlog: u32,
    pub tcp_tw_reuse: u32,
    pub ip_local_port_range: String,
    pub swappiness: u32,
    pub file_max: u64,
    pub netdev_max_backlog: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhpFpmTuningConfig {
    pub pm_type: String,
    pub max_children: u32,
    pub start_servers: Option<u32>,
    pub min_spare_servers: Option<u32>,
    pub max_spare_servers: Option<u32>,
    pub process_idle_timeout_secs: u32,
    pub max_requests: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppRuntimeHints {
    pub recommended_node_workers: u32,
    pub recommended_node_max_old_space_mb: u32,
    pub recommended_python_workers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizerProfile {
    pub profile_type: OptimizerProfileType,
    pub display_name: String,
    pub description: String,
    pub nginx: NginxTuningConfig,
    pub sysctl: SysctlTuningConfig,
    pub php_fpm: PhpFpmTuningConfig,
    pub hints: AppRuntimeHints,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OptimizerGetStatusRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizerStatusResponse {
    pub hardware: ServerHardwareSpecs,
    /// Additive verified planning facts; old payloads have no capacity snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<panel_core::capacity::CapacitySnapshot>,
    pub recommended_profile: OptimizerProfileType,
    pub active_profile: Option<OptimizerProfileType>,
    pub applied_at: Option<u64>,
    pub current_settings: OptimizerProfile,
    pub recommended_settings: OptimizerProfile,
    pub has_backup: bool,
    pub last_backup_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizerApplyRequest {
    pub profile: OptimizerProfileType,
    pub custom_tuning: Option<OptimizerProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizerApplyResponse {
    pub success: bool,
    pub applied_profile: OptimizerProfileType,
    pub message: String,
    pub backup_path: Option<String>,
    pub nginx_reloaded: bool,
    pub sysctl_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OptimizerRollbackRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptimizerRollbackResponse {
    pub success: bool,
    pub restored_profile: Option<OptimizerProfileType>,
    pub message: String,
    pub restored_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VersionInfo {
    pub version: String,
    pub proto_version: u32,
    pub os: String,
    pub arch: String,
}

/// Helper to wrap any async IO stream with length-delimited JSON RPC framing.
pub struct RpcStream<S> {
    inner: Framed<S, LengthDelimitedCodec>,
}

impl<S> RpcStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(stream: S) -> Self {
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(16 * 1024 * 1024) // 16 MB max frame
            .new_codec();
        Self {
            inner: Framed::new(stream, codec),
        }
    }

    pub async fn send_request(&mut self, req: &RpcRequest) -> Result<(), ProtoError> {
        let bytes = serde_json::to_vec(req)?;
        self.inner.send(Bytes::from(bytes)).await?;
        Ok(())
    }

    pub async fn recv_request(&mut self) -> Result<RpcRequest, ProtoError> {
        match self.inner.next().await {
            Some(Ok(bytes)) => {
                let req: RpcRequest = serde_json::from_slice(&bytes)?;
                if req.proto_version != PROTOCOL_VERSION {
                    return Err(ProtoError::VersionMismatch {
                        expected: PROTOCOL_VERSION,
                        got: req.proto_version,
                    });
                }
                Ok(req)
            }
            Some(Err(e)) => Err(ProtoError::Io(e)),
            None => Err(ProtoError::ConnectionClosed),
        }
    }

    pub async fn send_response(&mut self, res: &RpcResponse) -> Result<(), ProtoError> {
        let bytes = serde_json::to_vec(res)?;
        self.inner.send(Bytes::from(bytes)).await?;
        Ok(())
    }

    pub async fn recv_response(&mut self) -> Result<RpcResponse, ProtoError> {
        match self.inner.next().await {
            Some(Ok(bytes)) => {
                let res: RpcResponse = serde_json::from_slice(&bytes)?;
                Ok(res)
            }
            Some(Err(e)) => Err(ProtoError::Io(e)),
            None => Err(ProtoError::ConnectionClosed),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantPortBindingDto {
    pub site_id: u64,
    pub tenant_uid: u32,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftablesSyncRequest {
    pub bindings: Vec<TenantPortBindingDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftablesSyncResponse {
    pub success: bool,
    pub rules_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NftablesStatusRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftablesStatusDto {
    pub enabled: bool,
    pub table_name: String,
    pub active_rules_count: usize,
    pub protected_ports_count: usize,
    pub last_synced_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NftablesStatusResponse {
    pub status: NftablesStatusDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SystemDoctorCheckRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorCheckItemDto {
    pub id: String,
    pub name: String,
    pub category: String,
    pub status: String,
    pub message: String,
    pub remediation_hint: Option<String>,
    pub fixable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemDoctorCheckResponse {
    pub timestamp: u64,
    pub checks: Vec<DoctorCheckItemDto>,
    pub total_passed: usize,
    pub total_warnings: usize,
    pub total_failures: usize,
    pub overall_status: String,
    pub healthy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SystemDoctorFixRequest {
    pub target_checks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorRemediationResultDto {
    pub item_id: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemDoctorFixResponse {
    pub results: Vec<DoctorRemediationResultDto>,
    pub all_fixed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconcileSiteInputDto {
    pub site_id: i64,
    pub domain: String,
    pub username: String,
    pub uid: u32,
    pub kind: String,
    pub docroot: String,
    pub config_content: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SystemReconcileRequest {
    pub dry_run: bool,
    pub desired_sites: Vec<ReconcileSiteInputDto>,
    pub desired_nftables_ports: Vec<TenantPortBindingDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftItemDto {
    pub id: String,
    pub drift_type: String,
    pub resource_type: String,
    pub resource_id: String,
    pub description: String,
    pub desired_state: String,
    pub actual_state: String,
    pub remediation_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemReconcileResponse {
    pub timestamp: u64,
    pub total_resources_checked: usize,
    pub drifts: Vec<DriftItemDto>,
    pub is_clean: bool,
    pub reconciled_count: usize,
    pub failed_count: usize,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SystemBackupCreateRequest {
    pub output_filename: Option<String>,
    pub include_analytics: bool,
    pub db_path: Option<String>,
    pub analytics_db_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemBackupCreateResponse {
    pub success: bool,
    pub filename: String,
    pub filepath: String,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub files_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SystemBackupListRequest {
    pub backups_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemBackupSummaryDto {
    pub filename: String,
    pub filepath: String,
    pub created_at: u64,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub files_count: usize,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemBackupListResponse {
    pub backups: Vec<SystemBackupSummaryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemBackupRestoreRequest {
    pub backup_filepath: String,
    pub target_db_path: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemBackupRestoreResponse {
    pub success: bool,
    pub restored_files_count: usize,
    pub error: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn site_log_context_is_additive_and_does_not_accept_raw_paths() {
        let legacy = serde_json::json!({ "site_id": "site_1", "config_content": "server {}", "enable": true, "nginx_base_dir": null, "test_config": true, "reload": false });
        let request: SiteApplyRequest = serde_json::from_value(legacy.clone()).unwrap();
        assert!(request.log_context.is_none());
        assert_eq!(serde_json::to_value(request).unwrap(), legacy);
        let mut current = legacy;
        current["log_context"] =
            serde_json::json!({ "username": "p_alice", "domain": "example.com" });
        let request: SiteApplyRequest = serde_json::from_value(current.clone()).unwrap();
        assert_eq!(request.log_context.unwrap().domain, "example.com");
        current["log_context"]["log_dir"] = serde_json::json!("/etc/nginx");
        assert!(serde_json::from_value::<SiteApplyRequest>(current).is_err());
    }

    #[tokio::test]
    async fn test_rpc_serialization_ping() {
        let req = RpcRequest::new("req-1", RequestPayload::Ping);
        let serialized = serde_json::to_string(&req).expect("serialize");
        let deserialized: RpcRequest = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(req, deserialized);
    }

    #[tokio::test]
    async fn test_rpc_roundtrip_stream() {
        let (client_io, server_io) = tokio::io::duplex(1024);
        let mut client_rpc = RpcStream::new(client_io);
        let mut server_rpc = RpcStream::new(server_io);

        let client_task = tokio::spawn(async move {
            let req = RpcRequest::new("test-123", RequestPayload::Ping);
            client_rpc.send_request(&req).await.expect("send request");
            let res = client_rpc.recv_response().await.expect("recv response");
            assert_eq!(res.request_id, "test-123");
            assert_eq!(res.payload, ResponsePayload::Pong);
        });

        let server_task = tokio::spawn(async move {
            let req = server_rpc.recv_request().await.expect("recv request");
            assert_eq!(req.payload, RequestPayload::Ping);
            let res = RpcResponse::ok(req.request_id, ResponsePayload::Pong);
            server_rpc.send_response(&res).await.expect("send response");
        });

        let _ = tokio::try_join!(client_task, server_task).expect("roundtrip successful");
    }

    #[test]
    fn test_rpc_user_lifecycle_payloads() {
        let req = RpcRequest::new(
            "user-1",
            RequestPayload::UserCreate(UserCreateRequest {
                username: "p_alice".to_string(),
                uid: 10001,
                shell: Some("/usr/sbin/nologin".to_string()),
                home_dir: Some("/home/p_alice".to_string()),
                web_user: Some("www-data".to_string()),
            }),
        );
        let s = serde_json::to_string(&req).expect("serialize UserCreate");
        let de: RpcRequest = serde_json::from_str(&s).expect("deserialize UserCreate");
        assert_eq!(req, de);

        let res = RpcResponse::ok(
            "user-1",
            ResponsePayload::UserCreated(UserCreatedResponse {
                username: "p_alice".to_string(),
                uid: 10001,
                home_dir: "/home/p_alice".to_string(),
            }),
        );
        let s_res = serde_json::to_string(&res).expect("serialize UserCreated response");
        let de_res: RpcResponse = serde_json::from_str(&s_res).expect("deserialize UserCreated");
        assert_eq!(res, de_res);
    }

    #[test]
    fn test_rpc_slice_payloads() {
        let req = RpcRequest::new(
            "slice-1",
            RequestPayload::SliceApply(SliceApplyRequest {
                uid: 10001,
                memory_max: Some("1G".to_string()),
                memory_high: Some("800M".to_string()),
                cpu_quota: Some("150%".to_string()),
                tasks_max: Some(250),
                io_weight: Some(100),
            }),
        );
        let s = serde_json::to_string(&req).expect("serialize SliceApply");
        let de: RpcRequest = serde_json::from_str(&s).expect("deserialize SliceApply");
        assert_eq!(req, de);

        let res = RpcResponse::ok(
            "slice-1",
            ResponsePayload::SliceApplied(SliceAppliedResponse {
                uid: 10001,
                slice_name: "panel-u10001.slice".to_string(),
                dropin_path: "/etc/systemd/system/panel-u10001.slice.d/50-limits.conf".to_string(),
            }),
        );
        let s_res = serde_json::to_string(&res).expect("serialize SliceApplied");
        let de_res: RpcResponse = serde_json::from_str(&s_res).expect("deserialize SliceApplied");
        assert_eq!(res, de_res);

        let rem_req = RpcRequest::new(
            "slice-2",
            RequestPayload::SliceRemove(SliceRemoveRequest { uid: 10001 }),
        );
        let s_rem = serde_json::to_string(&rem_req).expect("serialize SliceRemove");
        let de_rem: RpcRequest = serde_json::from_str(&s_rem).expect("deserialize SliceRemove");
        assert_eq!(rem_req, de_rem);
    }

    #[test]
    fn test_rpc_quota_payloads() {
        let set_req = RpcRequest::new(
            "quota-1",
            RequestPayload::QuotaSet(QuotaSetRequest {
                uid: 10001,
                username: "p_alice".to_string(),
                disk_limit_bytes: 10 * 1024 * 1024 * 1024,
                inode_limit: 500_000,
                mount_point: Some("/home".to_string()),
            }),
        );
        let s_set = serde_json::to_string(&set_req).expect("serialize QuotaSet");
        let de_set: RpcRequest = serde_json::from_str(&s_set).expect("deserialize QuotaSet");
        assert_eq!(set_req, de_set);

        let get_res = RpcResponse::ok(
            "quota-2",
            ResponsePayload::QuotaGet(QuotaGetResponse {
                uid: 10001,
                username: "p_alice".to_string(),
                bytes_used: 1024 * 1024 * 50,
                bytes_limit: 10 * 1024 * 1024 * 1024,
                inodes_used: 1200,
                inodes_limit: 500_000,
                backend: "ext4".to_string(),
            }),
        );
        let s_get = serde_json::to_string(&get_res).expect("serialize QuotaGet");
        let de_get: RpcResponse = serde_json::from_str(&s_get).expect("deserialize QuotaGet");
        assert_eq!(get_res, de_get);

        let scan_res = RpcResponse::ok(
            "quota-3",
            ResponsePayload::QuotaScan(QuotaScanResponse {
                uid: 10001,
                username: "p_alice".to_string(),
                bytes_used: 2048,
                inodes_used: 10,
                scan_duration_ms: 15,
            }),
        );
        let s_scan = serde_json::to_string(&scan_res).expect("serialize QuotaScan");
        let de_scan: RpcResponse = serde_json::from_str(&s_scan).expect("deserialize QuotaScan");
        assert_eq!(scan_res, de_scan);
    }

    #[test]
    fn test_rpc_resource_read_payloads() {
        let req = RpcRequest::new(
            "res-1",
            RequestPayload::ResourceRead(ResourceReadRequest { uid: 10001 }),
        );
        let s_req = serde_json::to_string(&req).expect("serialize ResourceRead");
        let de_req: RpcRequest = serde_json::from_str(&s_req).expect("deserialize ResourceRead");
        assert_eq!(req, de_req);

        let res = RpcResponse::ok(
            "res-2",
            ResponsePayload::ResourceRead(ResourceReadResponse {
                metrics: SliceResourceMetrics {
                    uid: 10001,
                    slice_name: "panel-u10001.slice".to_string(),
                    exists: true,
                    cpu_usage_usec: 500_000,
                    cpu_user_usec: 400_000,
                    cpu_system_usec: 100_000,
                    cpu_nr_periods: 100,
                    cpu_nr_throttled: 2,
                    cpu_throttled_usec: 20_000,
                    memory_current_bytes: 64 * 1024 * 1024,
                    memory_peak_bytes: Some(128 * 1024 * 1024),
                    pids_current: 5,
                    pids_peak: Some(12),
                    io_read_bytes: 10 * 1024 * 1024,
                    io_write_bytes: 5 * 1024 * 1024,
                    io_read_ops: 256,
                    io_write_ops: 128,
                },
            }),
        );
        let s_res = serde_json::to_string(&res).expect("serialize ResourceRead response");
        let de_res: RpcResponse =
            serde_json::from_str(&s_res).expect("deserialize ResourceRead response");
        assert_eq!(res, de_res);
    }

    #[test]
    fn test_rpc_hardening_payloads() {
        let check_req = RpcRequest::new("h-1", RequestPayload::HardeningCheck);
        let s_chk = serde_json::to_string(&check_req).expect("serialize HardeningCheck");
        let de_chk: RpcRequest = serde_json::from_str(&s_chk).expect("deserialize HardeningCheck");
        assert_eq!(check_req, de_chk);

        let apply_req = RpcRequest::new(
            "h-2",
            RequestPayload::HardeningApply(HardeningApplyRequest {
                enable_hidepid: true,
                ensure_tenant_group: true,
            }),
        );
        let s_app = serde_json::to_string(&apply_req).expect("serialize HardeningApply");
        let de_app: RpcRequest = serde_json::from_str(&s_app).expect("deserialize HardeningApply");
        assert_eq!(apply_req, de_app);

        let status_res = RpcResponse::ok(
            "h-3",
            ResponsePayload::HardeningStatus(Box::new(HardeningStatus {
                cgroup_v2: PrerequisiteCheckItem {
                    name: "cgroup_v2".to_string(),
                    ok: true,
                    message: "cgroup v2 is active".to_string(),
                    details: None,
                },
                acl: PrerequisiteCheckItem {
                    name: "acl".to_string(),
                    ok: true,
                    message: "ACL is supported".to_string(),
                    details: None,
                },
                quota: PrerequisiteCheckItem {
                    name: "quota".to_string(),
                    ok: true,
                    message: "Quota is supported".to_string(),
                    details: None,
                },
                hidepid: PrerequisiteCheckItem {
                    name: "hidepid".to_string(),
                    ok: false,
                    message: "hidepid=2 not active".to_string(),
                    details: None,
                },
                tenant_group: PrerequisiteCheckItem {
                    name: "tenant_group".to_string(),
                    ok: true,
                    message: "Group exists".to_string(),
                    details: None,
                },
                overall_ok: false,
            })),
        );
        let s_stat = serde_json::to_string(&status_res).expect("serialize HardeningStatus");
        let de_stat: RpcResponse =
            serde_json::from_str(&s_stat).expect("deserialize HardeningStatus");
        assert_eq!(status_res, de_stat);
    }

    #[test]
    fn test_rpc_nginx_baseline_payloads() {
        let req = RpcRequest::new(
            "baseline-1",
            RequestPayload::NginxBaselineApply(NginxBaselineApplyRequest {
                base_dir: Some("/etc/panel/nginx".to_string()),
                log_dir: Some("/var/log/panel".to_string()),
                acme_dir: Some("/var/lib/panel/acme".to_string()),
                panel_api_port: Some(7700),
                panel_listen_port: Some(17890),
                test_config: true,
                reload: true,
            }),
        );
        let s = serde_json::to_string(&req).expect("serialize NginxBaselineApply");
        let de: RpcRequest = serde_json::from_str(&s).expect("deserialize NginxBaselineApply");
        assert_eq!(req, de);

        let res = RpcResponse::ok(
            "baseline-1",
            ResponsePayload::NginxBaselineApplied(NginxBaselineAppliedResponse {
                base_dir: "/etc/panel/nginx".to_string(),
                nginx_conf_path: "/etc/panel/nginx/nginx.conf".to_string(),
                conf_d_dir: "/etc/panel/nginx/conf.d".to_string(),
                snippets_dir: "/etc/panel/nginx/snippets".to_string(),
                sites_available_dir: "/etc/panel/nginx/sites-available".to_string(),
                sites_enabled_dir: "/etc/panel/nginx/sites-enabled".to_string(),
                log_dir: "/var/log/panel".to_string(),
                acme_dir: "/var/lib/panel/acme".to_string(),
                test_passed: true,
                reloaded: true,
            }),
        );
        let s_res = serde_json::to_string(&res).expect("serialize NginxBaselineApplied");
        let de_res: RpcResponse =
            serde_json::from_str(&s_res).expect("deserialize NginxBaselineApplied");
        assert_eq!(res, de_res);
    }

    #[test]
    fn test_rpc_site_logs_read_payloads() {
        let req = RpcRequest::new(
            "logs-1",
            RequestPayload::SiteLogsRead(SiteLogsReadRequest {
                username: "p_alice".to_string(),
                site_domain: "example.org".to_string(),
                log_type: "access".to_string(),
                lines: 50,
                query: Some("404".to_string()),
                log_base_dir: Some("/var/log/panel".to_string()),
                site_id: None,
            }),
        );
        let s = serde_json::to_string(&req).expect("serialize SiteLogsRead");
        let de: RpcRequest = serde_json::from_str(&s).expect("deserialize SiteLogsRead");
        assert_eq!(req, de);

        let res = RpcResponse::ok(
            "logs-1",
            ResponsePayload::SiteLogsRead(SiteLogsReadResponse {
                lines: vec!["{\"status\":404,\"request_uri\":\"/missing\"}".to_string()],
                total_matched: 1,
            }),
        );
        let s_res = serde_json::to_string(&res).expect("serialize SiteLogsRead response");
        let de_res: RpcResponse =
            serde_json::from_str(&s_res).expect("deserialize SiteLogsRead response");
        assert_eq!(res, de_res);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)]
    fn test_rpc_php_runtime_payloads() {
        let req_install = RpcRequest::new(
            "php-inst-1",
            RequestPayload::PhpRuntimeInstall(PhpRuntimeInstallRequest {
                version: "8.3".to_string(),
            }),
        );
        let s_req = serde_json::to_string(&req_install).expect("serialize PhpRuntimeInstall");
        let de_req: RpcRequest =
            serde_json::from_str(&s_req).expect("deserialize PhpRuntimeInstall");
        assert_eq!(req_install, de_req);

        let res_install = RpcResponse::ok(
            "php-inst-1",
            ResponsePayload::PhpRuntimeInstalled(PhpRuntimeActionResponse {
                version: "8.3".to_string(),
                success: true,
                masked: true,
                message: "PHP 8.3 installed and php8.3-fpm service masked".to_string(),
            }),
        );
        let s_res = serde_json::to_string(&res_install).expect("serialize PhpRuntimeInstalled");
        let de_res: RpcResponse =
            serde_json::from_str(&s_res).expect("deserialize PhpRuntimeInstalled");
        assert_eq!(res_install, de_res);

        let req_list = RpcRequest::new(
            "php-list-1",
            RequestPayload::PhpRuntimeList(PhpRuntimeListRequest {}),
        );
        let s_list = serde_json::to_string(&req_list).expect("serialize PhpRuntimeList");
        let de_list: RpcRequest =
            serde_json::from_str(&s_list).expect("deserialize PhpRuntimeList");
        assert_eq!(req_list, de_list);

        let res_list = RpcResponse::ok(
            "php-list-1",
            ResponsePayload::PhpRuntimeList(PhpRuntimeListResponse {
                ppa_configured: true,
                versions: vec![PhpVersionStatusDto {
                    version: "8.3".to_string(),
                    installed: true,
                    fpm_binary_exists: true,
                    cli_binary_exists: true,
                    service_masked: true,
                    status: "installed".to_string(),
                    fpm_binary_path: "/usr/sbin/php-fpm8.3".to_string(),
                    cli_binary_path: "/usr/bin/php8.3".to_string(),
                    service_name: "php8.3-fpm.service".to_string(),
                }],
            }),
        );
        let s_res_list =
            serde_json::to_string(&res_list).expect("serialize PhpRuntimeListResponse");
        let de_res_list: RpcResponse =
            serde_json::from_str(&s_res_list).expect("deserialize PhpRuntimeListResponse");
        assert_eq!(res_list, de_res_list);

        let req_ext = RpcRequest::new(
            "php-ext-1",
            RequestPayload::PhpExtensionToggle(PhpExtensionToggleRequest {
                version: "8.3".to_string(),
                extension: "redis".to_string(),
                enabled: true,
            }),
        );
        let s_ext = serde_json::to_string(&req_ext).expect("serialize PhpExtensionToggle");
        let de_ext: RpcRequest =
            serde_json::from_str(&s_ext).expect("deserialize PhpExtensionToggle");
        assert_eq!(req_ext, de_ext);

        let res_ext = RpcResponse::ok(
            "php-ext-1",
            ResponsePayload::PhpExtensionList(PhpExtensionListResponse {
                version: "8.3".to_string(),
                extensions: vec![PhpExtensionDto {
                    name: "redis".to_string(),
                    version: "8.3".to_string(),
                    package_name: "php8.3-redis".to_string(),
                    installed: true,
                    enabled: true,
                    is_base: false,
                    category: "Cache".to_string(),
                    description: "Driver Redis".to_string(),
                }],
            }),
        );
        let s_res_ext =
            serde_json::to_string(&res_ext).expect("serialize PhpExtensionListResponse");
        let de_res_ext: RpcResponse =
            serde_json::from_str(&s_res_ext).expect("deserialize PhpExtensionListResponse");
        assert_eq!(res_ext, de_res_ext);

        let req_fpm = RpcRequest::new(
            "fpm-req-1",
            RequestPayload::PhpFpmPoolApply(PhpFpmPoolApplyRequest {
                username: "p_alice".to_string(),
                uid: 10001,
                version: "8.3".to_string(),
                pool_name: "site-1".to_string(),
                docroot: "/home/p_alice/sites/example.com/public".to_string(),
                socket_path: None,
                max_children: Some(10),
                process_idle_timeout_secs: Some(10),
                max_requests: Some(500),
                open_basedir: None,
                disable_functions: None,
                settings: None,
                admin_ini: None,
                reload: true,
            }),
        );
        let s_fpm = serde_json::to_string(&req_fpm).expect("serialize PhpFpmPoolApplyRequest");
        let de_fpm: RpcRequest =
            serde_json::from_str(&s_fpm).expect("deserialize PhpFpmPoolApplyRequest");
        assert_eq!(req_fpm, de_fpm);

        let res_fpm = RpcResponse::ok(
            "fpm-res-1",
            ResponsePayload::PhpFpmPoolApply(PhpFpmPoolApplyResponse {
                pool_name: "site-1".to_string(),
                pool_conf_path: "/etc/panel/php/p_alice-8.3/pool.d/site-1.conf".to_string(),
                socket_path: "/run/panel/php/p_alice-8.3.sock".to_string(),
                reloaded: true,
            }),
        );
        let s_res_fpm = serde_json::to_string(&res_fpm).expect("serialize PhpFpmPoolApplyResponse");
        let de_res_fpm: RpcResponse =
            serde_json::from_str(&s_res_fpm).expect("deserialize PhpFpmPoolApplyResponse");
        assert_eq!(res_fpm, de_res_fpm);

        let req_hc = RpcRequest::new(
            "hc-req-1",
            RequestPayload::PhpFpmHealthCheck(PhpFpmHealthCheckRequest {
                username: "p_alice".to_string(),
                version: "8.3".to_string(),
                pool_name: "site-1".to_string(),
            }),
        );
        let s_hc = serde_json::to_string(&req_hc).expect("serialize PhpFpmHealthCheckRequest");
        let de_hc: RpcRequest =
            serde_json::from_str(&s_hc).expect("deserialize PhpFpmHealthCheckRequest");
        assert_eq!(req_hc, de_hc);

        let res_hc = RpcResponse::ok(
            "hc-res-1",
            ResponsePayload::PhpFpmHealthChecked(PhpFpmHealthCheckResponse {
                healthy: true,
                service_active: true,
                socket_exists: true,
                pool_exists: true,
                message: "Healthy".to_string(),
            }),
        );
        let s_res_hc = serde_json::to_string(&res_hc).expect("serialize PhpFpmHealthCheckResponse");
        let de_res_hc: RpcResponse =
            serde_json::from_str(&s_res_hc).expect("deserialize PhpFpmHealthCheckResponse");
        assert_eq!(res_hc, de_res_hc);

        // FPM status metrics parsing test with PHP-FPM json format
        let raw_fpm_json = r#"{
            "pool": "site-1",
            "process manager": "ondemand",
            "start time": 1728000000,
            "start since": 3600,
            "accepted conn": 150,
            "listen queue": 2,
            "max listen queue": 5,
            "listen queue len": 128,
            "idle processes": 3,
            "active processes": 1,
            "total processes": 4,
            "max active processes": 8,
            "max children reached": 0,
            "slow requests": 1
        }"#;
        let metrics: PhpFpmStatusMetrics =
            serde_json::from_str(raw_fpm_json).expect("parse metrics");
        assert_eq!(metrics.pool, "site-1");
        assert_eq!(metrics.process_manager, "ondemand");
        assert_eq!(metrics.listen_queue, 2);
        assert_eq!(metrics.active_processes, 1);
        assert_eq!(metrics.idle_processes, 3);
        assert_eq!(metrics.slow_requests, 1);

        // Pool status RPC
        let req_status = RpcRequest::new(
            "status-req-1",
            RequestPayload::PhpFpmPoolStatus(PhpFpmPoolStatusRequest {
                username: "p_alice".to_string(),
                version: "8.3".to_string(),
                pool_name: "site-1".to_string(),
            }),
        );
        let s_status = serde_json::to_string(&req_status).unwrap();
        let de_status: RpcRequest = serde_json::from_str(&s_status).unwrap();
        assert_eq!(req_status, de_status);

        // Log read RPC
        let req_logs = RpcRequest::new(
            "log-req-1",
            RequestPayload::PhpLogsRead(PhpLogsReadRequest {
                username: "p_alice".to_string(),
                version: "8.3".to_string(),
                pool_name: Some("site-1".to_string()),
                log_type: "error".to_string(),
                lines: 50,
                query: Some("warning".to_string()),
                log_base_dir: None,
            }),
        );
        let s_logs = serde_json::to_string(&req_logs).unwrap();
        let de_logs: RpcRequest = serde_json::from_str(&s_logs).unwrap();
        assert_eq!(req_logs, de_logs);

        // PTY Open RPC test
        let req_pty_open = RpcRequest::new(
            "pty-open-1",
            RequestPayload::PtyOpen(PtyOpenRequest {
                username: "p_alice".to_string(),
                uid: 10001,
                gid: 10001,
                cwd: Some("sites/example.com".to_string()),
                rows: 24,
                cols: 80,
                term: Some("xterm-256color".to_string()),
                max_sessions: Some(5),
                idle_timeout_secs: Some(900),
            }),
        );
        let s_pty_open = serde_json::to_string(&req_pty_open).unwrap();
        let de_pty_open: RpcRequest = serde_json::from_str(&s_pty_open).unwrap();
        assert_eq!(req_pty_open, de_pty_open);

        let res_pty_open = RpcResponse::ok(
            "pty-open-1",
            ResponsePayload::PtyOpen(PtyOpenResponse {
                session_id: "test-sess-1".to_string(),
                username: "p_alice".to_string(),
                pid: 12345,
                cwd: "/home/p_alice/sites/example.com".to_string(),
                rows: 24,
                cols: 80,
                created_at: 1728000000,
            }),
        );
        let s_res_open = serde_json::to_string(&res_pty_open).unwrap();
        let de_res_open: RpcResponse = serde_json::from_str(&s_res_open).unwrap();
        assert_eq!(res_pty_open, de_res_open);

        // PTY Input RPC test
        let req_pty_in = RpcRequest::new(
            "pty-in-1",
            RequestPayload::PtyInput(PtyInputRequest {
                session_id: "test-sess-1".to_string(),
                data: b"ls -la\n".to_vec(),
            }),
        );
        let s_pty_in = serde_json::to_string(&req_pty_in).unwrap();
        let de_pty_in: RpcRequest = serde_json::from_str(&s_pty_in).unwrap();
        assert_eq!(req_pty_in, de_pty_in);

        // PTY Resize RPC test
        let req_pty_resize = RpcRequest::new(
            "pty-res-1",
            RequestPayload::PtyResize(PtyResizeRequest {
                session_id: "test-sess-1".to_string(),
                rows: 40,
                cols: 120,
            }),
        );
        let s_pty_resize = serde_json::to_string(&req_pty_resize).unwrap();
        let de_pty_resize: RpcRequest = serde_json::from_str(&s_pty_resize).unwrap();
        assert_eq!(req_pty_resize, de_pty_resize);

        // Node.js Runtime RPC tests
        let req_node_list = RpcRequest::new(
            "node-list-1",
            RequestPayload::NodeRuntimeList(NodeRuntimeListRequest {
                base_dir: Some("/opt/panel/runtimes/node".to_string()),
            }),
        );
        let s_node_list = serde_json::to_string(&req_node_list).unwrap();
        let de_node_list: RpcRequest = serde_json::from_str(&s_node_list).unwrap();
        assert_eq!(req_node_list, de_node_list);

        let req_node_install = RpcRequest::new(
            "node-install-1",
            RequestPayload::NodeRuntimeInstall(NodeRuntimeInstallRequest {
                version: "22.14.0".to_string(),
                mirror_url: Some("https://nodejs.org/dist".to_string()),
                offline_tarball_path: None,
                offline_checksum: None,
                enable_corepack: true,
                base_dir: None,
            }),
        );
        let s_node_install = serde_json::to_string(&req_node_install).unwrap();
        let de_node_install: RpcRequest = serde_json::from_str(&s_node_install).unwrap();
        assert_eq!(req_node_install, de_node_install);

        let res_node_action = RpcResponse::ok(
            "node-install-1",
            ResponsePayload::NodeRuntimeInstalled(NodeRuntimeActionResponse {
                version: "22.14.0".to_string(),
                success: true,
                path: "/opt/panel/runtimes/node/22.14.0".to_string(),
                message: "Node.js v22.14.0 berhasil dipasang".to_string(),
            }),
        );
        let s_node_res = serde_json::to_string(&res_node_action).unwrap();
        let de_node_res: RpcResponse = serde_json::from_str(&s_node_res).unwrap();
        assert_eq!(res_node_action, de_node_res);

        // Node.js Site RPC tests
        let mut env_vars = std::collections::BTreeMap::new();
        env_vars.insert("NODE_ENV".to_string(), "production".to_string());
        env_vars.insert("DB_HOST".to_string(), "127.0.0.1".to_string());

        let req_site_apply = RpcRequest::new(
            "node-site-1",
            RequestPayload::NodeSiteApply(NodeSiteApplyRequest {
                site_id: "site-42".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                node_version: "22.14.0".to_string(),
                app_dir: "/home/p_alice/sites/node.test/app".to_string(),
                port: 20042,
                entrypoint: NodeSiteEntrypoint::NodeScript("server.js".to_string()),
                env_vars,
                auto_restart: true,
                env_file_path: None,
                unit_path: None,
                base_runtime_dir: None,
            }),
        );
        let s_site_apply = serde_json::to_string(&req_site_apply).unwrap();
        let de_site_apply: RpcRequest = serde_json::from_str(&s_site_apply).unwrap();
        assert_eq!(req_site_apply, de_site_apply);

        let res_site_apply = RpcResponse::ok(
            "node-site-1",
            ResponsePayload::NodeSiteApplied(NodeSiteApplyResponse {
                site_id: "site-42".to_string(),
                service_name: "site-site-42.service".to_string(),
                unit_path: "/etc/systemd/system/site-site-42.service".to_string(),
                env_path: "/etc/panel/env/site-site-42.env".to_string(),
                port: 20042,
                active: true,
                message: "Service site-site-42.service started".to_string(),
            }),
        );
        let s_res_apply = serde_json::to_string(&res_site_apply).unwrap();
        let de_res_apply: RpcResponse = serde_json::from_str(&s_res_apply).unwrap();
        assert_eq!(res_site_apply, de_res_apply);

        let req_site_action = RpcRequest::new(
            "node-action-1",
            RequestPayload::NodeSiteAction(NodeSiteActionRequest {
                site_id: "site-42".to_string(),
                action: NodeSiteActionType::Restart,
            }),
        );
        let s_site_action = serde_json::to_string(&req_site_action).unwrap();
        let de_site_action: RpcRequest = serde_json::from_str(&s_site_action).unwrap();
        assert_eq!(req_site_action, de_site_action);

        let req_site_status = RpcRequest::new(
            "node-status-1",
            RequestPayload::NodeSiteStatus(NodeSiteStatusRequest {
                site_id: "site-42".to_string(),
                port: Some(20042),
                probe_path: Some("/health".to_string()),
            }),
        );
        let s_site_status = serde_json::to_string(&req_site_status).unwrap();
        let de_site_status: RpcRequest = serde_json::from_str(&s_site_status).unwrap();
        assert_eq!(req_site_status, de_site_status);

        let res_site_status = RpcResponse::ok(
            "node-status-1",
            ResponsePayload::NodeSiteStatus(NodeSiteStatusResponse {
                site_id: "site-42".to_string(),
                service_name: "site-site-42.service".to_string(),
                active: true,
                sub_state: "running".to_string(),
                pid: Some(12345),
                memory_bytes: Some(64 * 1024 * 1024),
                cpu_usage_usec: Some(100_000),
                uptime_secs: Some(3600),
                healthy: true,
                probe_status_code: Some(200),
                probe_message: Some("OK".to_string()),
            }),
        );
        let s_res_status = serde_json::to_string(&res_site_status).unwrap();
        let de_res_status: RpcResponse = serde_json::from_str(&s_res_status).unwrap();
        assert_eq!(res_site_status, de_res_status);

        let req_site_job = RpcRequest::new(
            "node-job-1",
            RequestPayload::NodeSiteJob(NodeSiteJobRequest {
                site_id: "site-42".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                node_version: "22.14.0".to_string(),
                app_dir: "/home/p_alice/sites/node.test/app".to_string(),
                job_type: NodeSiteJobType::NpmInstall,
                timeout_secs: Some(300),
                base_runtime_dir: None,
            }),
        );
        let s_site_job = serde_json::to_string(&req_site_job).unwrap();
        let de_site_job: RpcRequest = serde_json::from_str(&s_site_job).unwrap();
        assert_eq!(req_site_job, de_site_job);

        // Python Proto Serialization Tests
        let mut py_env = std::collections::BTreeMap::new();
        py_env.insert(
            "DJANGO_SETTINGS_MODULE".to_string(),
            "mysite.settings".to_string(),
        );
        let req_py_apply = RpcRequest::new(
            "py-apply-1",
            RequestPayload::PythonSiteApply(PythonSiteApplyRequest {
                site_id: "site_py42".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                python_version: "3.12.9".to_string(),
                app_dir: "/home/p_alice/sites/django.test/app".to_string(),
                venv_dir: "/home/p_alice/sites/django.test/.venv".to_string(),
                socket_path: "/run/panel/python/site_py42.sock".to_string(),
                framework: PythonSiteFramework::Django,
                server_type: PythonSiteServer::GunicornWsgi,
                entrypoint: PythonSiteEntrypoint::Wsgi("mysite.wsgi:application".to_string()),
                workers: 3,
                env_vars: py_env,
                auto_restart: true,
                env_file_path: None,
                unit_path: None,
                base_runtime_dir: None,
            }),
        );
        let s_py_apply = serde_json::to_string(&req_py_apply).unwrap();
        let de_py_apply: RpcRequest = serde_json::from_str(&s_py_apply).unwrap();
        assert_eq!(req_py_apply, de_py_apply);

        let res_py_status = RpcResponse::ok(
            "py-status-1",
            ResponsePayload::PythonSiteStatus(PythonSiteStatusResponse {
                site_id: "site_py42".to_string(),
                service_name: "site-site_py42.service".to_string(),
                active: true,
                sub_state: "running".to_string(),
                pid: Some(54321),
                memory_bytes: Some(128 * 1024 * 1024),
                cpu_usage_usec: Some(250_000),
                uptime_secs: Some(7200),
                socket_exists: true,
                healthy: true,
                probe_message: Some("Socket probe OK".to_string()),
            }),
        );
        let s_py_status = serde_json::to_string(&res_py_status).unwrap();
        let de_py_status: RpcResponse = serde_json::from_str(&s_py_status).unwrap();
        assert_eq!(res_py_status, de_py_status);

        let req_py_job = RpcRequest::new(
            "py-job-1",
            RequestPayload::PythonSiteJob(PythonSiteJobRequest {
                site_id: "site_py42".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                python_version: "3.12.9".to_string(),
                app_dir: "/home/p_alice/sites/django.test/app".to_string(),
                venv_dir: "/home/p_alice/sites/django.test/.venv".to_string(),
                job_type: PythonSiteJobType::UvPipInstallRequirements { req_file: None },
                timeout_secs: Some(300),
                base_runtime_dir: None,
            }),
        );
        let s_py_job = serde_json::to_string(&req_py_job).unwrap();
        let de_py_job: RpcRequest = serde_json::from_str(&s_py_job).unwrap();
        assert_eq!(req_py_job, de_py_job);

        let req_probe = RpcRequest::new(
            "proxy-probe-1",
            RequestPayload::ProxyUpstreamProbe(ProxyUpstreamProbeRequest {
                upstream_url: "http://127.0.0.1:8080".to_string(),
                timeout_ms: Some(2000),
            }),
        );
        let s_probe = serde_json::to_string(&req_probe).unwrap();
        let de_probe: RpcRequest = serde_json::from_str(&s_probe).unwrap();
        assert_eq!(req_probe, de_probe);

        let res_probe = RpcResponse::ok(
            "proxy-probe-1",
            ResponsePayload::ProxyUpstreamProbed(ProxyUpstreamProbeResponse {
                healthy: true,
                status_code: Some(200),
                latency_ms: 15,
                message: "Upstream healthy".to_string(),
            }),
        );
        let s_res_probe = serde_json::to_string(&res_probe).unwrap();
        let de_res_probe: RpcResponse = serde_json::from_str(&s_res_probe).unwrap();
        assert_eq!(res_probe, de_res_probe);

        // P8-T1 SiteEnv payloads roundtrip test
        let mut env_map = std::collections::BTreeMap::new();
        env_map.insert(
            "DATABASE_URL".to_string(),
            "postgres://user:pass@localhost:5432/app".to_string(),
        );
        let req_env_enc = RpcRequest::new(
            "env-enc-1",
            RequestPayload::SiteEnvEncrypt(SiteEnvEncryptRequest {
                site_id: "site-1".to_string(),
                env_vars: env_map.clone(),
            }),
        );
        let s_env_enc = serde_json::to_string(&req_env_enc).unwrap();
        let de_env_enc: RpcRequest = serde_json::from_str(&s_env_enc).unwrap();
        assert_eq!(req_env_enc, de_env_enc);

        let res_env_enc = RpcResponse::ok(
            "env-enc-1",
            ResponsePayload::SiteEnvEncrypted(SiteEnvEncryptResponse {
                site_id: "site-1".to_string(),
                encrypted_vars: env_map.clone(),
            }),
        );
        let s_res_env_enc = serde_json::to_string(&res_env_enc).unwrap();
        let de_res_env_enc: RpcResponse = serde_json::from_str(&s_res_env_enc).unwrap();
        assert_eq!(res_env_enc, de_res_env_enc);

        let req_env_app = RpcRequest::new(
            "env-app-1",
            RequestPayload::SiteEnvApply(SiteEnvApplyRequest {
                site_id: "site-1".to_string(),
                username: "u1001".to_string(),
                env_file_path: Some("/etc/panel/env/site-1.env".to_string()),
                encrypted_vars: env_map,
            }),
        );
        let s_env_app = serde_json::to_string(&req_env_app).unwrap();
        let de_env_app: RpcRequest = serde_json::from_str(&s_env_app).unwrap();
        assert_eq!(req_env_app, de_env_app);

        // P8-T2 Cron payloads roundtrip tests
        let req_cron_apply = RpcRequest::new(
            "cron-app-1",
            RequestPayload::CronJobApply(CronJobApplyRequest {
                site_id: "site-1".to_string(),
                job_id: 42,
                name: "Backup DB".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                command: "php artisan schedule:run".to_string(),
                schedule: "*/15 * * * *".to_string(),
                systemd_calendar: "*-*-* *:00/15:00".to_string(),
                work_dir: Some("/home/p_alice/sites/app".to_string()),
                timeout_secs: 1800,
                enabled: true,
                unit_dir: None,
                env_file_path: None,
            }),
        );
        let s_cron_apply = serde_json::to_string(&req_cron_apply).unwrap();
        let de_cron_apply: RpcRequest = serde_json::from_str(&s_cron_apply).unwrap();
        assert_eq!(req_cron_apply, de_cron_apply);

        let res_cron_apply = RpcResponse::ok(
            "cron-app-1",
            ResponsePayload::CronJobApplied(CronJobApplyResponse {
                site_id: "site-1".to_string(),
                job_id: 42,
                service_name: "site-site-1-cron-42.service".to_string(),
                timer_name: "site-site-1-cron-42.timer".to_string(),
                enabled: true,
                success: true,
                message: "Cron timer enabled".to_string(),
            }),
        );
        let s_res_cron_app = serde_json::to_string(&res_cron_apply).unwrap();
        let de_res_cron_app: RpcResponse = serde_json::from_str(&s_res_cron_app).unwrap();
        assert_eq!(res_cron_apply, de_res_cron_app);

        let req_cron_run = RpcRequest::new(
            "cron-run-1",
            RequestPayload::CronJobRunNow(CronJobRunNowRequest {
                site_id: "site-1".to_string(),
                job_id: 42,
                username: "p_alice".to_string(),
                uid: 10001,
                command: "php artisan schedule:run".to_string(),
                work_dir: None,
                timeout_secs: 300,
                env_file_path: None,
                unit_dir: None,
            }),
        );
        let s_cron_run = serde_json::to_string(&req_cron_run).unwrap();
        let de_cron_run: RpcRequest = serde_json::from_str(&s_cron_run).unwrap();
        assert_eq!(req_cron_run, de_cron_run);

        let res_cron_run = RpcResponse::ok(
            "cron-run-1",
            ResponsePayload::CronJobRunNow(CronJobRunNowResponse {
                site_id: "site-1".to_string(),
                job_id: 42,
                exit_code: 0,
                stdout: "Running scheduled tasks... Done".to_string(),
                stderr: String::new(),
                duration_ms: 1250,
                success: true,
            }),
        );
        let s_res_cron_run = serde_json::to_string(&res_cron_run).unwrap();
        let de_res_cron_run: RpcResponse = serde_json::from_str(&s_res_cron_run).unwrap();
        assert_eq!(res_cron_run, de_res_cron_run);

        // P8-T4 Git Deploy payloads roundtrip tests
        let req_deploy_key = RpcRequest::new(
            "git-key-1",
            RequestPayload::SiteDeployKeyGenerate(SiteDeployKeyGenerateRequest {
                site_id: "site-1".to_string(),
                username: "p_alice".to_string(),
                key_dir: Some("/etc/panel/keys".to_string()),
            }),
        );
        let s_deploy_key = serde_json::to_string(&req_deploy_key).unwrap();
        let de_deploy_key: RpcRequest = serde_json::from_str(&s_deploy_key).unwrap();
        assert_eq!(req_deploy_key, de_deploy_key);

        let res_deploy_key = RpcResponse::ok(
            "git-key-1",
            ResponsePayload::SiteDeployKeyGenerated(SiteDeployKeyGenerateResponse {
                site_id: "site-1".to_string(),
                public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... deploy@vpanel".to_string(),
                private_key_path: "/etc/panel/keys/site_site-1_deploy_key".to_string(),
                public_key_path: "/etc/panel/keys/site_site-1_deploy_key.pub".to_string(),
                success: true,
            }),
        );
        let s_res_key = serde_json::to_string(&res_deploy_key).unwrap();
        let de_res_key: RpcResponse = serde_json::from_str(&s_res_key).unwrap();
        assert_eq!(res_deploy_key, de_res_key);

        let req_git_deploy = RpcRequest::new(
            "git-deploy-1",
            RequestPayload::SiteGitDeploy(SiteGitDeployRequest {
                site_id: "site-1".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                repo_url: "git@github.com:org/repo.git".to_string(),
                branch: "main".to_string(),
                commit_hash: None,
                base_dir: "/home/p_alice/sites/example.com".to_string(),
                docroot_subpath: Some("dist".to_string()),
                build_cmd: Some("npm run build".to_string()),
                post_deploy_cmd: Some("./post-deploy.sh".to_string()),
                release_id: "20261003-120000-abcd".to_string(),
                keep_releases: 5,
                deploy_key_path: Some("/etc/panel/keys/site_site-1_deploy_key".to_string()),
                env_file_path: Some("/etc/panel/env/site-1.env".to_string()),
            }),
        );
        let s_git_deploy = serde_json::to_string(&req_git_deploy).unwrap();
        let de_git_deploy: RpcRequest = serde_json::from_str(&s_git_deploy).unwrap();
        assert_eq!(req_git_deploy, de_git_deploy);

        let res_git_deploy = RpcResponse::ok(
            "git-deploy-1",
            ResponsePayload::SiteGitDeployed(SiteGitDeployResponse {
                site_id: "site-1".to_string(),
                release_id: "20261003-120000-abcd".to_string(),
                commit_hash: "abcdef123456".to_string(),
                commit_author: "Alice <alice@example.com>".to_string(),
                commit_message: "feat: initial commit".to_string(),
                duration_ms: 3500,
                success: true,
                log: "Cloning... Built successfully.".to_string(),
                current_symlink_target:
                    "/home/p_alice/sites/example.com/releases/20261003-120000-abcd/dist".to_string(),
            }),
        );
        let s_res_deploy = serde_json::to_string(&res_git_deploy).unwrap();
        let de_res_deploy: RpcResponse = serde_json::from_str(&s_res_deploy).unwrap();
        assert_eq!(res_git_deploy, de_res_deploy);

        let req_git_rollback = RpcRequest::new(
            "git-rollback-1",
            RequestPayload::SiteGitRollback(SiteGitRollbackRequest {
                site_id: "site-1".to_string(),
                username: "p_alice".to_string(),
                uid: 10001,
                base_dir: "/home/p_alice/sites/example.com".to_string(),
                target_release_id: "20261003-110000-prev".to_string(),
            }),
        );
        let s_git_rollback = serde_json::to_string(&req_git_rollback).unwrap();
        let de_git_rollback: RpcRequest = serde_json::from_str(&s_git_rollback).unwrap();
        assert_eq!(req_git_rollback, de_git_rollback);

        let res_git_rollback = RpcResponse::ok(
            "git-rollback-1",
            ResponsePayload::SiteGitRollbacked(SiteGitRollbackResponse {
                site_id: "site-1".to_string(),
                release_id: "20261003-110000-prev".to_string(),
                success: true,
                log: "Symlink swapped to previous release".to_string(),
                current_symlink_target:
                    "/home/p_alice/sites/example.com/releases/20261003-110000-prev".to_string(),
            }),
        );
        let s_res_rollback = serde_json::to_string(&res_git_rollback).unwrap();
        let de_res_rollback: RpcResponse = serde_json::from_str(&s_res_rollback).unwrap();
        assert_eq!(res_git_rollback, de_res_rollback);

        // P8-T5 Database payloads roundtrip tests
        let req_db_create = RpcRequest::new(
            "db-create-1",
            RequestPayload::DatabaseCreate(DatabaseCreateRequest {
                engine: "mariadb".to_string(),
                name: "p_alice_app".to_string(),
                db_user: "p_alice_user".to_string(),
                password: "SecurePassword123!".to_string(),
                remote_access: false,
                allowed_hosts: "localhost".to_string(),
                collation: "utf8mb4_unicode_ci".to_string(),
                character_set: "utf8mb4".to_string(),
            }),
        );
        let s_db_create = serde_json::to_string(&req_db_create).unwrap();
        let de_db_create: RpcRequest = serde_json::from_str(&s_db_create).unwrap();
        assert_eq!(req_db_create, de_db_create);

        let res_db_create = RpcResponse::ok(
            "db-create-1",
            ResponsePayload::DatabaseCreated(DatabaseCreateResponse {
                engine: "mariadb".to_string(),
                name: "p_alice_app".to_string(),
                db_user: "p_alice_user".to_string(),
                success: true,
                message: "Database created".to_string(),
            }),
        );
        let s_res_create = serde_json::to_string(&res_db_create).unwrap();
        let de_res_create: RpcResponse = serde_json::from_str(&s_res_create).unwrap();
        assert_eq!(res_db_create, de_res_create);

        let req_db_server = RpcRequest::new(
            "db-server-1",
            RequestPayload::DatabaseServerStatus(DatabaseServerStatusRequest {
                engines: vec!["mariadb".to_string(), "postgresql".to_string()],
            }),
        );
        let s_db_server = serde_json::to_string(&req_db_server).unwrap();
        let de_db_server: RpcRequest = serde_json::from_str(&s_db_server).unwrap();
        assert_eq!(req_db_server, de_db_server);

        let res_db_server = RpcResponse::ok(
            "db-server-1",
            ResponsePayload::DatabaseServerStatus(DatabaseServerStatusResponse {
                servers: vec![
                    DatabaseServerStatusItem {
                        engine: "mariadb".to_string(),
                        is_installed: true,
                        is_running: true,
                        version: Some("11.4.2".to_string()),
                        port: 3306,
                        socket_path: Some("/run/mysqld/mysqld.sock".to_string()),
                    },
                    DatabaseServerStatusItem {
                        engine: "postgresql".to_string(),
                        is_installed: false,
                        is_running: false,
                        version: None,
                        port: 5432,
                        socket_path: None,
                    },
                ],
            }),
        );
        let s_res_server = serde_json::to_string(&res_db_server).unwrap();
        let de_res_server: RpcResponse = serde_json::from_str(&s_res_server).unwrap();
        assert_eq!(res_db_server, de_res_server);

        let req_db_export = RpcRequest::new(
            "db-export-1",
            RequestPayload::DatabaseExport(DatabaseExportRequest {
                engine: "mariadb".to_string(),
                name: "p_alice_app".to_string(),
                db_user: Some("p_alice_user".to_string()),
                export_file_path: "/home/p_alice/backup.sql".to_string(),
                tenant_username: "p_alice".to_string(),
                tenant_uid: 10001,
            }),
        );
        let s_db_export = serde_json::to_string(&req_db_export).unwrap();
        let de_db_export: RpcRequest = serde_json::from_str(&s_db_export).unwrap();
        assert_eq!(req_db_export, de_db_export);

        let res_db_export = RpcResponse::ok(
            "db-export-1",
            ResponsePayload::DatabaseExported(DatabaseExportResponse {
                engine: "mariadb".to_string(),
                name: "p_alice_app".to_string(),
                file_path: "/home/p_alice/backup.sql".to_string(),
                size_bytes: 1048576,
                success: true,
                message: "Export completed".to_string(),
            }),
        );
        let s_res_export = serde_json::to_string(&res_db_export).unwrap();
        let de_res_export: RpcResponse = serde_json::from_str(&s_res_export).unwrap();
        assert_eq!(res_db_export, de_res_export);
    }

    #[test]
    fn test_rpc_backup_payloads() {
        let req_create = RpcRequest::new(
            "bk-create-1",
            RequestPayload::BackupCreate(BackupCreateRequest {
                site_id: 1,
                tenant_username: "p_alice".to_string(),
                tenant_uid: 10001,
                site_dir: "/home/p_alice/sites/mysite".to_string(),
                backup_dir: "/home/p_alice/backups".to_string(),
                filename: "backup_mysite_1700000000.tar.zst".to_string(),
                include_files: true,
                include_databases: true,
                databases: vec![BackupDatabaseInfo {
                    engine: "mariadb".to_string(),
                    name: "p_alice_db".to_string(),
                    username: "p_alice_u".to_string(),
                    password: Some("secret123".to_string()),
                }],
                format: "tar.zst".to_string(),
            }),
        );
        let s_req = serde_json::to_string(&req_create).unwrap();
        let de_req: RpcRequest = serde_json::from_str(&s_req).unwrap();
        assert_eq!(req_create, de_req);

        let res_create = RpcResponse::ok(
            "bk-create-1",
            ResponsePayload::BackupCreated(BackupCreateResponse {
                success: true,
                size_bytes: 5242880,
                checksum_sha256: "aabbcc112233".to_string(),
                filepath: "/home/p_alice/backups/backup_mysite_1700000000.tar.zst".to_string(),
                error: None,
            }),
        );
        let s_res = serde_json::to_string(&res_create).unwrap();
        let de_res: RpcResponse = serde_json::from_str(&s_res).unwrap();
        assert_eq!(res_create, de_res);

        let req_restore = RpcRequest::new(
            "bk-restore-1",
            RequestPayload::BackupRestore(BackupRestoreRequest {
                site_id: 1,
                tenant_username: "p_alice".to_string(),
                tenant_uid: 10001,
                backup_filepath: "/home/p_alice/backups/backup_mysite_1700000000.tar.zst"
                    .to_string(),
                target_site_dir: "/home/p_alice/sites/mysite".to_string(),
                restore_files: true,
                restore_databases: true,
                databases: vec![],
            }),
        );
        let s_res_r = serde_json::to_string(&req_restore).unwrap();
        let de_res_r: RpcRequest = serde_json::from_str(&s_res_r).unwrap();
        assert_eq!(req_restore, de_res_r);
    }

    #[test]
    fn test_rpc_optimizer_payloads() {
        let req_status = RpcRequest::new(
            "opt-status-1",
            RequestPayload::OptimizerGetStatus(OptimizerGetStatusRequest {}),
        );
        let s_status = serde_json::to_string(&req_status).unwrap();
        let de_status: RpcRequest = serde_json::from_str(&s_status).unwrap();
        assert_eq!(req_status, de_status);

        let req_apply = RpcRequest::new(
            "opt-apply-1",
            RequestPayload::OptimizerApply(OptimizerApplyRequest {
                profile: OptimizerProfileType::Standard,
                custom_tuning: None,
            }),
        );
        let s_apply = serde_json::to_string(&req_apply).unwrap();
        let de_apply: RpcRequest = serde_json::from_str(&s_apply).unwrap();
        assert_eq!(req_apply, de_apply);

        let res_apply = RpcResponse::ok(
            "opt-apply-1",
            ResponsePayload::OptimizerApplied(OptimizerApplyResponse {
                success: true,
                applied_profile: OptimizerProfileType::Standard,
                message: "Server tuning applied successfully".to_string(),
                backup_path: Some("/var/lib/panel/optimizer/backup/snapshot_1.tar".to_string()),
                nginx_reloaded: true,
                sysctl_applied: true,
            }),
        );
        let s_res_app = serde_json::to_string(&res_apply).unwrap();
        let de_res_app: RpcResponse = serde_json::from_str(&s_res_app).unwrap();
        assert_eq!(res_apply, de_res_app);

        let req_rollback = RpcRequest::new(
            "opt-rb-1",
            RequestPayload::OptimizerRollback(OptimizerRollbackRequest {}),
        );
        let s_rb = serde_json::to_string(&req_rollback).unwrap();
        let de_rb: RpcRequest = serde_json::from_str(&s_rb).unwrap();
        assert_eq!(req_rollback, de_rb);

        // P10-T4 Nftables payloads roundtrip tests
        let req_nft_sync = RpcRequest::new(
            "nft-sync-1",
            RequestPayload::NftablesSync(NftablesSyncRequest {
                bindings: vec![TenantPortBindingDto {
                    site_id: 101,
                    tenant_uid: 1001,
                    port: 20005,
                }],
            }),
        );
        let s_nft_sync = serde_json::to_string(&req_nft_sync).unwrap();
        let de_nft_sync: RpcRequest = serde_json::from_str(&s_nft_sync).unwrap();
        assert_eq!(req_nft_sync, de_nft_sync);

        let res_nft_sync = RpcResponse::ok(
            "nft-sync-1",
            ResponsePayload::NftablesSynced(NftablesSyncResponse {
                success: true,
                rules_count: 3,
                message: "Rules synced".to_string(),
            }),
        );
        let s_res_nft = serde_json::to_string(&res_nft_sync).unwrap();
        let de_res_nft: RpcResponse = serde_json::from_str(&s_res_nft).unwrap();
        assert_eq!(res_nft_sync, de_res_nft);

        // P10-T5 Doctor, Reconcile, and Backup payloads roundtrip tests
        let req_doc = RpcRequest::new(
            "doc-req-1",
            RequestPayload::SystemDoctorCheck(SystemDoctorCheckRequest {}),
        );
        let s_doc = serde_json::to_string(&req_doc).unwrap();
        let de_doc: RpcRequest = serde_json::from_str(&s_doc).unwrap();
        assert_eq!(req_doc, de_doc);

        let res_doc = RpcResponse::ok(
            "doc-req-1",
            ResponsePayload::SystemDoctorChecked(SystemDoctorCheckResponse {
                timestamp: 1720000000,
                checks: vec![DoctorCheckItemDto {
                    id: "nginx_syntax".to_string(),
                    name: "Nginx Syntax".to_string(),
                    category: "services".to_string(),
                    status: "pass".to_string(),
                    message: "Syntax is ok".to_string(),
                    remediation_hint: None,
                    fixable: false,
                }],
                total_passed: 1,
                total_warnings: 0,
                total_failures: 0,
                overall_status: "pass".to_string(),
                healthy: true,
            }),
        );
        let s_res_doc = serde_json::to_string(&res_doc).unwrap();
        let de_res_doc: RpcResponse = serde_json::from_str(&s_res_doc).unwrap();
        assert_eq!(res_doc, de_res_doc);

        let req_reconcile = RpcRequest::new(
            "rec-1",
            RequestPayload::SystemReconcile(SystemReconcileRequest {
                dry_run: true,
                desired_sites: vec![],
                desired_nftables_ports: vec![],
            }),
        );
        let s_rec = serde_json::to_string(&req_reconcile).unwrap();
        let de_rec: RpcRequest = serde_json::from_str(&s_rec).unwrap();
        assert_eq!(req_reconcile, de_rec);

        let res_reconcile = RpcResponse::ok(
            "rec-1",
            ResponsePayload::SystemReconciled(SystemReconcileResponse {
                timestamp: 1720000000,
                total_resources_checked: 10,
                drifts: vec![],
                is_clean: true,
                reconciled_count: 0,
                failed_count: 0,
                success: true,
                message: "Synced".to_string(),
            }),
        );
        let s_res_rec = serde_json::to_string(&res_reconcile).unwrap();
        let de_res_rec: RpcResponse = serde_json::from_str(&s_res_rec).unwrap();
        assert_eq!(res_reconcile, de_res_rec);

        let req_backup = RpcRequest::new(
            "bk-1",
            RequestPayload::SystemBackupCreate(SystemBackupCreateRequest {
                output_filename: Some("panel-backup-1.tar.gz".to_string()),
                include_analytics: true,
                db_path: None,
                analytics_db_path: None,
            }),
        );
        let s_bk = serde_json::to_string(&req_backup).unwrap();
        let de_bk: RpcRequest = serde_json::from_str(&s_bk).unwrap();
        assert_eq!(req_backup, de_bk);
    }
}
