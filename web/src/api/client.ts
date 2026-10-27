import { parseSystemInsights, parseSystemTimeseries, parseRetention, parseMaintenance, parseIntegrity, retentionUpdatePayload } from './analyticsContracts';
import { parseDatabaseList } from './databaseContracts';
import {
  AuditLogItem,
  AuditLogListResponse,
  AuditLogQuery,
  CreatePlanRequest,
  CreateTenantRequest,
  CsrfResponse,
  HardeningApplyRequest,
  HardeningApplyResponse,
  HardeningStatus,
  HealthResponse,
  JobItem,
  JobListResponse,
  JobProgressEvent,
  LoginResponse,
  Plan,
  PlanListResponse,
  PlanResponse,
  SystemInfoResponse,
  TenantDetailResponse,
  TenantListItem,
  TenantListResponse,
  TenantResourceUsage,
  TwoFactorEnableResponse,
  TwoFactorSetupResponse,
  TwoFactorStatusResponse,
  UpdatePlanRequest,
  User,
  Site,
  SiteDomain,
  CreateSiteRequest,
  AddDomainRequest,
  UpdateDomainRequest,
  SiteMessageResponse,
  SiteOverviewResponse,
  SiteQuickActionResponse,
  SslStatus,
  IssueSslRequest,
  CustomCertRequest,
  SslActionResponse,
  SiteLogsResponse,
  PhpRuntimeListApiResponse,
  PhpExtensionActionApiResponse,
  PhpExtensionListApiResponse,
  SitePhpExtensionsResponse,
  SwitchPhpVersionResponse,
  SitePhpSettingsResponse,
  UpdateSitePhpSettingsInput,
  SitePhpStatusResponse,
  SitePhpLogsResponse,
  SiteRulesConfig,
  HashPasswordResponse,
  ValidateSnippetResponse,
  SiteGitConfig,
  UpsertGitConfigRequest,
  TriggerDeployRequest,
  RollbackDeployRequest,
  SiteDeployment,
  GenerateDeployKeyResponse,
  DatabaseServer,
  DatabaseEngine,
  DatabaseItem,
  DatabaseListResponse,
  CreateDatabaseRequest,
  CreateDatabaseResponse,
  RotatePasswordRequest,
  RotatePasswordResponse,
  UpdateRemoteAccessRequest,
  DatabaseActionResponse,
  ExportDatabaseRequest,
  ImportDatabaseRequest,
  BackupItem,
  BackupListResponse,
  BackupSchedule,
  CreateBackupRequest,
  RestoreBackupRequest,
  UpdateScheduleRequest,
  ListNotificationsResponse,
  NotificationSettings,
  UpdateNotificationSettingsRequest,
  TestSmtpRequest,
  TestSmtpResponse,
  EvaluateAlertsResponse,
  AnalyticsTimeRangeQuery,
  AnalyticsBreakdownsQuery,
  AnalyticsSlowlogQuery,
  AnalyticsExportQuery,
  SiteAnalyticsOverviewResponse,
  SiteTimeseriesResponse,
  SiteBreakdownsResponse,
  SiteAppMetricsResponse,
  SiteSlowlogResponse,
  SystemTimeseriesResponse,
  SiteInsightsResponse,
  SystemInsightsResponse,
  SecurityOverviewResponse,
  BannedIpDto,
  BanIpRequest,
  UnbanIpResponse,
  AnalyticsRetentionResponse,
  UpdateRetentionRequest,
  TriggerMaintenanceRequest,
  RetentionMaintenanceReportDto,
  IntegrityCheckReportDto,
  OptimizerStatusResponseDto,
  PackageDemand,
  ApplyOptimizerRequestDto,
  OptimizerActionResponseDto,
  PanelMemoryProfileReport,
  SecurityHardeningOverviewResponse,
  NftablesSyncPayload,
  NftablesSyncResponseDto,
  DoctorReport,
  ReconcileReport,
  SystemBackupSummary,
  CreateSystemBackupPayload,
  RestoreSystemBackupPayload,
  SystemBackupVerificationResult,
} from '../types/api';
import type { FsListResponse, TrashListResponse } from '../types/files';

const API_BASE = '';
let cachedCsrfToken: string | null = null;

export function setCsrfToken(token: string | null) {
  cachedCsrfToken = token;
}

export function getCsrfToken(): string | null {
  return cachedCsrfToken;
}

export function errorMessage(body: unknown, fallback: string): string {
  if (body && typeof body === 'object') {
    const payload = body as Record<string, unknown>;
    for (const key of ['detail', 'message', 'error', 'title']) {
      if (typeof payload[key] === 'string' && payload[key].trim()) return payload[key];
    }
  }
  return fallback;
}

export async function request<T>(
  url: string,
  options: RequestInit = {}
): Promise<T> {
  const headers = new Headers(options.headers || {});
  
  if (!headers.has('Content-Type') && options.body && typeof options.body === 'string') {
    headers.set('Content-Type', 'application/json');
  }

  // Attach CSRF token on mutating methods if available
  const method = (options.method || 'GET').toUpperCase();
  if (['POST', 'PUT', 'DELETE', 'PATCH'].includes(method) && cachedCsrfToken) {
    headers.set('X-CSRF-Token', cachedCsrfToken);
  }

  const res = await fetch(`${API_BASE}${url}`, {
    ...options,
    headers,
    credentials: 'include',
  });

  if (!res.ok) {
    if (res.status === 401) {
      setCsrfToken(null);
      if (typeof window !== 'undefined' && !url.startsWith('/api/v1/auth/setup') && url !== '/api/v1/auth/login') window.dispatchEvent(new Event('panel:session-expired'));
    }
    let message = res.statusText || 'Request failed';
    try {
      const text = await res.text();
      try { message = errorMessage(JSON.parse(text), message); }
      catch { if (text && !text.trimStart().startsWith('<')) message = text; }
    } catch { /* Keep the HTTP fallback if reading the response fails. */ }
    const err = new Error(message);
    (err as unknown as { status: number }).status = res.status;
    throw err;
  }

  // Handle empty bodies (e.g. 204 No Content)
  if (res.status === 204) {
    return {} as T;
  }

  return res.json();
}

// System & Health
export async function fetchHealth(): Promise<HealthResponse> {
  return request<HealthResponse>('/healthz');
}

export async function fetchSystemInfo(): Promise<SystemInfoResponse> {
  return request<SystemInfoResponse>('/api/v1/system/info');
}
export interface HostMetricsResponse {
  state: 'waiting' | 'ready' | 'failed' | 'stale';
  error: string | null;
  sampling_interval_secs: number;
  sample: {timestamp:number;cpu_percent:number;cpu_cores:number;memory_used_bytes:number;memory_total_bytes:number;disk_used_bytes:number;disk_total_bytes:number;net_rx_bytes_sec:number;net_tx_bytes_sec:number;load1:number} | null;
}
export function fetchSystemMetrics(): Promise<HostMetricsResponse> { return request('/api/v1/system/metrics'); }

// Authentication & Session
export async function fetchCsrfToken(): Promise<string> {
  const res = await request<CsrfResponse>('/api/v1/auth/csrf');
  setCsrfToken(res.csrf_token);
  return res.csrf_token;
}

export async function fetchMe(): Promise<User> {
  const res = await request<LoginResponse | User>('/api/v1/auth/me');
  if ('user' in res && res.user) {
    if ('csrf_token' in res && res.csrf_token) setCsrfToken(res.csrf_token);
    return res.user;
  }
  return res as User;
}

export async function login(
  username: string,
  password: string,
  twoFactorCode?: string
): Promise<LoginResponse> {
  const body: Record<string, string> = { username, password };
  if (twoFactorCode) {
    body.code = twoFactorCode;
  }

  const res = await request<LoginResponse>('/api/v1/auth/login', {
    method: 'POST',
    body: JSON.stringify(body),
  });

  if (res.csrf_token) {
    setCsrfToken(res.csrf_token);
  }

  return res;
}

export async function logout(): Promise<{ message: string }> {
  const res = await request<{ message: string }>('/api/v1/auth/logout', {
    method: 'POST',
  });
  setCsrfToken(null);
  return res;
}

export async function changePassword(
  currentPassword: string,
  newPassword?: string,
  username?: string,
): Promise<LoginResponse> {
  const res = await request<LoginResponse>('/api/v1/auth/password', {
    method: 'POST',
    body: JSON.stringify({ current_password: currentPassword, new_password: newPassword, username }),
  });
  if (res.csrf_token) setCsrfToken(res.csrf_token);
  return res;
}

// Two-Factor Authentication (2FA)
export async function setup2FA(): Promise<TwoFactorSetupResponse> {
  return request<TwoFactorSetupResponse>('/api/v1/auth/2fa/setup', {
    method: 'POST',
  });
}

export async function enable2FA(code: string): Promise<TwoFactorEnableResponse> {
  return request<TwoFactorEnableResponse>('/api/v1/auth/2fa/enable', {
    method: 'POST',
    body: JSON.stringify({ code }),
  });
}

export async function disable2FA(code: string): Promise<{ message: string }> {
  return request<{ message: string }>('/api/v1/auth/2fa/disable', {
    method: 'POST',
    body: JSON.stringify({ code }),
  });
}

export async function fetch2FAStatus(): Promise<TwoFactorStatusResponse> {
  return request<TwoFactorStatusResponse>('/api/v1/auth/2fa/status');
}

// Audit Logs
export async function fetchAuditLogs(
  query: AuditLogQuery = {},
  signal?: AbortSignal,
): Promise<AuditLogListResponse> {
  const params = new URLSearchParams();
  if (query.category) params.set('category', query.category);
  if (query.q) params.set('q', query.q);
  if (query.page) params.set('page', query.page.toString());
  if (query.per_page) params.set('per_page', query.per_page.toString());
  if (query.action) params.set('action', query.action);
  if (query.actor) params.set('actor', query.actor);
  if (query.target) params.set('target', query.target);
  if (query.status) params.set('status', query.status);

  const qs = params.toString() ? `?${params.toString()}` : '';
  return request<AuditLogListResponse>(`/api/v1/audit/logs${qs}`, { signal });
}

export async function fetchAuditLogDetail(id: number): Promise<AuditLogItem> {
  return request<AuditLogItem>(`/api/v1/audit/logs/${id}`);
}

// Background Jobs
export async function fetchJobs(
  query: { status?: string; page?: number; per_page?: number } = {}
): Promise<JobListResponse> {
  const params = new URLSearchParams();
  if (query.page) params.set('page', query.page.toString());
  if (query.per_page) params.set('per_page', query.per_page.toString());
  if (query.status) params.set('status', query.status);

  const qs = params.toString() ? `?${params.toString()}` : '';
  return request<JobListResponse>(`/api/v1/jobs${qs}`);
}

export async function fetchJobDetail(id: string): Promise<JobItem> {
  return request<JobItem>(`/api/v1/jobs/${encodeURIComponent(id)}`);
}

export async function cancelJob(id: string): Promise<JobItem> {
  return request<JobItem>(`/api/v1/jobs/${id}/cancel`, {
    method: 'POST',
  });
}

export function subscribeJobEvents(
  jobId: string,
  onEvent: (event: JobProgressEvent) => void,
  onError?: (err: Event) => void
): () => void {
  const eventSource = new EventSource(`${API_BASE}/api/v1/jobs/${jobId}/events`, {
    withCredentials: true,
  });

  eventSource.addEventListener('progress', (e: MessageEvent) => {
    try {
      const data: JobProgressEvent = JSON.parse(e.data);
      onEvent(data);
      if (['succeeded', 'failed', 'cancelled'].includes(data.status)) {
        eventSource.close();
      }
    } catch {
      // parse error
    }
  });

  if (onError) {
    eventSource.onerror = (err) => {
      onError(err);
    };
  }

  return () => {
    eventSource.close();
  };
}

export function fetchJobLogs(id: string, after = 0): Promise<JobProgressEvent[]> {
  return request<JobProgressEvent[]>(`/api/v1/jobs/${encodeURIComponent(id)}/logs?after=${after}`);
}

// Plans API
export async function fetchPlans(): Promise<Plan[]> {
  const res = await request<PlanListResponse>('/api/v1/plans');
  return res.plans;
}

export async function fetchPlan(id: number): Promise<Plan> {
  const res = await request<PlanResponse>(`/api/v1/plans/${id}`);
  return res.plan;
}

export async function createPlan(data: CreatePlanRequest): Promise<Plan> {
  const res = await request<PlanResponse>('/api/v1/plans', {
    method: 'POST',
    body: JSON.stringify(data),
  });
  return res.plan;
}

export async function updatePlan(id: number, data: UpdatePlanRequest): Promise<Plan> {
  const res = await request<PlanResponse>(`/api/v1/plans/${id}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
  return res.plan;
}

export async function deletePlan(id: number): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>(`/api/v1/plans/${id}`, {
    method: 'DELETE',
  });
}

// Tenants API
export async function fetchTenants(query?: { search?: string; status?: string }): Promise<TenantListItem[]> {
  const params = new URLSearchParams();
  if (query?.search) params.append('search', query.search);
  if (query?.status) params.append('status', query.status);
  const q = params.toString() ? `?${params.toString()}` : '';
  const res = await request<TenantListResponse>(`/api/v1/tenants${q}`);
  return res.tenants;
}

export async function fetchTenantDetail(id: number): Promise<TenantDetailResponse> {
  return request<TenantDetailResponse>(`/api/v1/tenants/${id}`);
}

export async function createTenant(data: CreateTenantRequest): Promise<TenantDetailResponse> {
  return request<TenantDetailResponse>('/api/v1/tenants', {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function fetchTenantResources(id: number): Promise<TenantResourceUsage> {
  return request<TenantResourceUsage>(`/api/v1/tenants/${id}/resources`);
}

export async function suspendTenant(id: number): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>(`/api/v1/tenants/${id}/suspend`, {
    method: 'POST',
  });
}

export async function unsuspendTenant(id: number): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>(`/api/v1/tenants/${id}/unsuspend`, {
    method: 'POST',
  });
}

export async function deleteTenant(id: number, purge?: boolean): Promise<{ success: boolean; message: string }> {
  const q = purge ? '?purge=true' : '';
  return request<{ success: boolean; message: string }>(`/api/v1/tenants/${id}${q}`, {
    method: 'DELETE',
  });
}

export async function impersonateTenant(id: number): Promise<User> {
  const res = await request<{ user: User; csrf_token: string; impersonated_by: string }>(`/api/v1/tenants/${id}/impersonate`, {
    method: 'POST',
  });
  if (res.csrf_token) {
    setCsrfToken(res.csrf_token);
  }
  return res.user;
}

export async function stopImpersonating(): Promise<void> {
  await request<{ success: boolean; message: string }>('/api/v1/auth/stop-impersonating', {
    method: 'POST',
  });
}

// System Hardening API
export async function fetchHardeningStatus(): Promise<HardeningStatus> {
  return request<HardeningStatus>('/api/v1/system/hardening');
}

export async function applyHardening(data: HardeningApplyRequest): Promise<HardeningApplyResponse> {
  return request<HardeningApplyResponse>('/api/v1/system/hardening/apply', {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

// ==========================================
// Sites API
// ==========================================

export async function fetchSites(query?: { user_id?: number }): Promise<Site[]> {
  const params = new URLSearchParams();
  if (query?.user_id) {
    params.set('user_id', query.user_id.toString());
  }
  const queryString = params.toString() ? `?${params.toString()}` : '';
  return request<Site[]>(`/api/v1/sites${queryString}`);
}

export async function fetchSite(id: number): Promise<Site> {
  return request<Site>(`/api/v1/sites/${id}`);
}

export async function fetchSiteOverview(siteId: number): Promise<SiteOverviewResponse> {
  return request<SiteOverviewResponse>(`/api/v1/sites/${siteId}/overview`);
}

export async function executeSiteQuickAction(
  siteId: number,
  action: 'start' | 'stop' | 'restart' | 'reload' | 'clear_cache' | string
): Promise<SiteQuickActionResponse> {
  return request<SiteQuickActionResponse>(`/api/v1/sites/${siteId}/action`, {
    method: 'POST',
    body: JSON.stringify({ action }),
  });
}

export async function createSite(data: CreateSiteRequest): Promise<Site> {
  return request<Site>('/api/v1/sites', {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function deleteSite(id: number): Promise<SiteMessageResponse> {
  return request<SiteMessageResponse>(`/api/v1/sites/${id}`, {
    method: 'DELETE',
  });
}

export async function enableSite(id: number): Promise<SiteMessageResponse> {
  return request<SiteMessageResponse>(`/api/v1/sites/${id}/enable`, {
    method: 'POST',
  });
}

export async function disableSite(id: number): Promise<SiteMessageResponse> {
  return request<SiteMessageResponse>(`/api/v1/sites/${id}/disable`, {
    method: 'POST',
  });
}

export async function setForceHttps(id: number, forceHttps: boolean): Promise<Site> {
  return request<Site>(`/api/v1/sites/${id}/force-https`, {
    method: 'POST',
    body: JSON.stringify({ force_https: forceHttps }),
  });
}

// ==========================================
// Site Domains API
// ==========================================

export async function fetchSiteDomains(siteId: number): Promise<SiteDomain[]> {
  return request<SiteDomain[]>(`/api/v1/sites/${siteId}/domains`);
}

export async function addSiteDomain(siteId: number, data: AddDomainRequest): Promise<SiteDomain> {
  return request<SiteDomain>(`/api/v1/sites/${siteId}/domains`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function updateSiteDomain(
  siteId: number,
  domainId: number,
  data: UpdateDomainRequest
): Promise<SiteDomain> {
  return request<SiteDomain>(`/api/v1/sites/${siteId}/domains/${domainId}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function deleteSiteDomain(
  siteId: number,
  domainId: number
): Promise<SiteMessageResponse> {
  return request<SiteMessageResponse>(`/api/v1/sites/${siteId}/domains/${domainId}`, {
    method: 'DELETE',
  });
}

// ==========================================
// SSL & Certificate API
// ==========================================

export async function fetchSiteSsl(siteId: number): Promise<SslStatus> {
  return request<SslStatus>(`/api/v1/sites/${siteId}/ssl`);
}

export async function issueSiteSsl(
  siteId: number,
  data: IssueSslRequest
): Promise<SslActionResponse> {
  return request<SslActionResponse>(`/api/v1/sites/${siteId}/ssl/issue`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function generateSelfSignedSsl(siteId: number): Promise<SslActionResponse> {
  return request<SslActionResponse>(`/api/v1/sites/${siteId}/ssl/self-signed`, {
    method: 'POST',
  });
}

export async function uploadCustomSsl(
  siteId: number,
  data: CustomCertRequest
): Promise<SslActionResponse> {
  return request<SslActionResponse>(`/api/v1/sites/${siteId}/ssl/custom`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function renewSiteSsl(siteId: number): Promise<SslActionResponse> {
  return request<SslActionResponse>(`/api/v1/sites/${siteId}/ssl/renew`, {
    method: 'POST',
  });
}

export async function deleteSiteSsl(siteId: number): Promise<SslActionResponse> {
  return request<SslActionResponse>(`/api/v1/sites/${siteId}/ssl`, {
    method: 'DELETE',
  });
}

// ==========================================
// Site Logs API
// ==========================================

export async function fetchSiteLogs(
  siteId: number,
  logType: 'access' | 'error' | 'php_error' | 'php_slow' | string,
  lines?: number,
  query?: string
): Promise<SiteLogsResponse> {
  const params = new URLSearchParams();
  params.set('log_type', logType);
  if (lines) params.set('lines', lines.toString());
  if (query) params.set('query', query);
  return request<SiteLogsResponse>(`/api/v1/sites/${siteId}/logs?${params.toString()}`);
}

// ==========================================
// PHP Runtime API
// ==========================================

export async function fetchPhpVersions(): Promise<PhpRuntimeListApiResponse> {
  return request<PhpRuntimeListApiResponse>('/api/v1/php/versions');
}

export async function setupPhpRepo(): Promise<JobItem> {
  return request<JobItem>('/api/v1/php/repo/setup', {
    method: 'POST',
  });
}

export async function installPhpVersion(version: string): Promise<JobItem> {
  return request<JobItem>(`/api/v1/php/versions/${encodeURIComponent(version)}/install`, {
    method: 'POST',
  });
}

export async function removePhpVersion(version: string): Promise<JobItem> {
  return request<JobItem>(`/api/v1/php/versions/${encodeURIComponent(version)}/remove`, {
    method: 'POST',
  });
}

export async function fetchPhpExtensions(version: string): Promise<PhpExtensionListApiResponse> {
  return request<PhpExtensionListApiResponse>(`/api/v1/php/versions/${encodeURIComponent(version)}/extensions`);
}

export async function installPhpExtension(version: string, extension: string): Promise<JobItem> {
  return request<JobItem>(
    `/api/v1/php/versions/${encodeURIComponent(version)}/extensions/${encodeURIComponent(extension)}/install`,
    {
      method: 'POST',
    }
  );
}

export async function togglePhpExtension(
  version: string,
  extension: string,
  enabled: boolean
): Promise<PhpExtensionActionApiResponse> {
  return request<PhpExtensionActionApiResponse>(
    `/api/v1/php/versions/${encodeURIComponent(version)}/extensions/${encodeURIComponent(extension)}/toggle`,
    {
      method: 'POST',
      body: JSON.stringify({ enabled }),
    }
  );
}

export async function removePhpExtension(version: string, extension: string): Promise<JobItem> {
  return request<JobItem>(
    `/api/v1/php/versions/${encodeURIComponent(version)}/extensions/${encodeURIComponent(extension)}/remove`,
    {
      method: 'POST',
    }
  );
}

export async function fetchSitePhpExtensions(siteId: number): Promise<SitePhpExtensionsResponse> {
  return request<SitePhpExtensionsResponse>(`/api/v1/sites/${siteId}/php/extensions`);
}

export async function updateSitePhpExtensions(
  siteId: number,
  extensions: string[]
): Promise<SitePhpExtensionsResponse> {
  return request<SitePhpExtensionsResponse>(`/api/v1/sites/${siteId}/php/extensions`, {
    method: 'PUT',
    body: JSON.stringify({ extensions }),
  });
}

export async function switchSitePhpVersion(
  siteId: number,
  version: string
): Promise<SwitchPhpVersionResponse> {
  return request<SwitchPhpVersionResponse>(`/api/v1/sites/${siteId}/php/version`, {
    method: 'POST',
    body: JSON.stringify({ version }),
  });
}

export async function fetchSitePhpSettings(siteId: number): Promise<SitePhpSettingsResponse> {
  return request<SitePhpSettingsResponse>(`/api/v1/sites/${siteId}/php/settings`);
}

export async function updateSitePhpSettings(
  siteId: number,
  data: UpdateSitePhpSettingsInput
): Promise<SitePhpSettingsResponse> {
  return request<SitePhpSettingsResponse>(`/api/v1/sites/${siteId}/php/settings`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function fetchSitePhpStatus(siteId: number): Promise<SitePhpStatusResponse> {
  return request<SitePhpStatusResponse>(`/api/v1/sites/${siteId}/php/status`);
}

export async function fetchSitePhpLogs(
  siteId: number,
  logType: 'error' | 'slow' = 'error',
  lines?: number,
  query?: string
): Promise<SitePhpLogsResponse> {
  const params = new URLSearchParams();
  params.set('type', logType);
  if (lines) params.set('lines', lines.toString());
  if (query) params.set('q', query);
  return request<SitePhpLogsResponse>(`/api/v1/sites/${siteId}/php/logs?${params.toString()}`);
}

// ==========================================
// File Manager API
// ==========================================

export async function fetchFileList(params: {
  path?: string;
  offset?: number;
  limit?: number;
  sort_by?: string;
  sort_order?: string;
  show_hidden?: boolean;
  search_query?: string;
  extension?: string;
}): Promise<FsListResponse> {
  const q = new URLSearchParams();
  if (params.path) q.set('path', params.path);
  if (params.offset !== undefined) q.set('offset', params.offset.toString());
  if (params.limit !== undefined) q.set('limit', params.limit.toString());
  if (params.sort_by) q.set('sort_by', params.sort_by);
  if (params.sort_order) q.set('sort_order', params.sort_order);
  if (params.show_hidden !== undefined) q.set('show_hidden', params.show_hidden ? 'true' : 'false');
  if (params.search_query) q.set('search_query', params.search_query);
  if (params.extension) q.set('extension', params.extension);

  const qs = q.toString() ? `?${q.toString()}` : '';
  return request<FsListResponse>(`/api/v1/files/list${qs}`);
}

export async function createFolder(
  path: string,
  createParents = true
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/mkdir', {
    method: 'POST',
    body: JSON.stringify({ path, create_parents: createParents }),
  });
}

export async function renameFile(
  from: string,
  to: string,
  overwrite = false
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/rename', {
    method: 'POST',
    body: JSON.stringify({ from, to, overwrite }),
  });
}

export async function moveFile(
  from: string,
  to: string,
  overwrite = false
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/move', {
    method: 'POST',
    body: JSON.stringify({ from, to, overwrite }),
  });
}

export async function copyFile(
  from: string,
  to: string,
  overwrite = false,
  asJob = false
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/copy', {
    method: 'POST',
    body: JSON.stringify({ from, to, overwrite, as_job: asJob }),
  });
}

export async function moveToTrash(
  path: string
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/trash', {
    method: 'POST',
    body: JSON.stringify({ path }),
  });
}

export async function fetchTrashList(): Promise<TrashListResponse> {
  return request<TrashListResponse>('/api/v1/files/trash');
}

export async function restoreTrash(
  trashId: string
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/trash/restore', {
    method: 'POST',
    body: JSON.stringify({ trash_id: trashId }),
  });
}

export async function emptyTrash(): Promise<{
  success: boolean;
  message: string;
  deleted_count: number;
  reclaimed_bytes: number;
}> {
  return request<{
    success: boolean;
    message: string;
    deleted_count: number;
    reclaimed_bytes: number;
  }>('/api/v1/files/trash/empty', {
    method: 'DELETE',
  });
}

export async function deleteFilePermanent(
  path: string
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>('/api/v1/files/permanent', {
    method: 'DELETE',
    body: JSON.stringify({ path }),
  });
}

export async function compressArchive(
  sources: string[],
  targetArchive: string,
  format = 'zip'
): Promise<{ success: boolean; message: string; job_id?: string }> {
  return request<{ success: boolean; message: string; job_id?: string }>(
    '/api/v1/files/archive/compress',
    {
      method: 'POST',
      body: JSON.stringify({ sources, target_archive: targetArchive, format }),
    }
  );
}

export async function extractArchive(
  archivePath: string,
  targetDir = ''
): Promise<{ success: boolean; message: string; job_id?: string }> {
  return request<{ success: boolean; message: string; job_id?: string }>(
    '/api/v1/files/archive/extract',
    {
      method: 'POST',
      body: JSON.stringify({ archive_path: archivePath, target_dir: targetDir }),
    }
  );
}

export async function initUpload(
  path: string,
  totalSize: number,
  fileHash?: string
): Promise<{ upload_id: string; chunk_size: number }> {
  return request<{ upload_id: string; chunk_size: number }>('/api/v1/files/upload/init', {
    method: 'POST',
    body: JSON.stringify({ path, total_size: totalSize, file_hash: fileHash }),
  });
}

export async function uploadChunk(
  uploadId: string,
  offset: number,
  chunk: Blob
): Promise<{ success: boolean; current_offset: number; complete: boolean }> {
  const res = await fetch(`/api/v1/files/upload/${uploadId}/chunk?offset=${offset}`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/octet-stream',
    },
    body: chunk,
    credentials: 'include',
  });

  if (!res.ok) {
    const err = (await res.json().catch(() => ({}))) as { detail?: string; message?: string };
    throw new Error(err.detail || err.message || `Unggah gagal dengan status ${res.status}`);
  }

  return res.json() as Promise<{ success: boolean; current_offset: number; complete: boolean }>;
}

export async function createTerminalSession(
  payload: import('../types/terminal').CreateTerminalSessionPayload
): Promise<import('../types/terminal').TerminalSession> {
  return request<import('../types/terminal').TerminalSession>('/api/v1/terminal/sessions', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function fetchTerminalSessions(
  tenant?: string
): Promise<{ sessions: import('../types/terminal').TerminalSession[] }> {
  const query = tenant ? `?tenant=${encodeURIComponent(tenant)}` : '';
  return request<{ sessions: import('../types/terminal').TerminalSession[] }>(
    `/api/v1/terminal/sessions${query}`
  );
}

export async function getTerminalSession(
  sessionId: string
): Promise<import('../types/terminal').TerminalSession> {
  return request<import('../types/terminal').TerminalSession>(
    `/api/v1/terminal/sessions/${encodeURIComponent(sessionId)}`
  );
}

export async function closeTerminalSession(
  sessionId: string
): Promise<import('../types/terminal').TerminalCloseResponse> {
  return request<import('../types/terminal').TerminalCloseResponse>(
    `/api/v1/terminal/sessions/${encodeURIComponent(sessionId)}`,
    {
      method: 'DELETE',
    }
  );
}

export async function resizeTerminalSession(
  sessionId: string,
  cols: number,
  rows: number
): Promise<import('../types/terminal').TerminalResizeResponse> {
  return request<import('../types/terminal').TerminalResizeResponse>(
    `/api/v1/terminal/sessions/${encodeURIComponent(sessionId)}/resize`,
    {
      method: 'POST',
      body: JSON.stringify({ cols, rows }),
    }
  );
}

// SSH Key Management API (Phase 6 - P6-T5)
export async function fetchSshKeys(
  query?: { user_id?: number; tenant?: string }
): Promise<import('../types/api').SshKeyItem[]> {
  const params = new URLSearchParams();
  if (query?.user_id) params.set('user_id', String(query.user_id));
  if (query?.tenant) params.set('tenant', query.tenant);
  const q = params.toString() ? `?${params.toString()}` : '';
  return request<import('../types/api').SshKeyItem[]>(`/api/v1/ssh-keys${q}`);
}

export async function createSshKey(
  data: import('../types/api').CreateSshKeyRequest
): Promise<import('../types/api').SshKeyItem> {
  return request<import('../types/api').SshKeyItem>('/api/v1/ssh-keys', {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function deleteSshKey(
  id: number
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>(
    `/api/v1/ssh-keys/${id}`,
    {
      method: 'DELETE',
    }
  );
}

export async function validateSshKey(
  publicKey: string
): Promise<import('../types/api').ValidateSshKeyResponse> {
  return request<import('../types/api').ValidateSshKeyResponse>(
    '/api/v1/ssh-keys/validate',
    {
      method: 'POST',
      body: JSON.stringify({ public_key: publicKey }),
    }
  );
}

// Shell Policy & SSH Drop-in Management (Phase 6 - P6-T6)
export async function fetchTenantShellPolicy(
  id: number
): Promise<import('../types/api').TenantShellPolicyResponse> {
  return request<import('../types/api').TenantShellPolicyResponse>(
    `/api/v1/tenants/${id}/shell-policy`
  );
}

export async function updateTenantShellPolicy(
  id: number,
  shellPolicy: string | null
): Promise<import('../types/api').TenantShellPolicyResponse> {
  return request<import('../types/api').TenantShellPolicyResponse>(
    `/api/v1/tenants/${id}/shell-policy`,
    {
      method: 'PUT',
      body: JSON.stringify({ shell_policy: shellPolicy }),
    }
  );
}

export async function applySshDropin(
  customContent?: string | null
): Promise<import('../types/api').SshDropinApplyResponse> {
  return request<import('../types/api').SshDropinApplyResponse>(
    '/api/v1/ssh/dropin/apply',
    {
      method: 'POST',
      body: JSON.stringify({ custom_content: customContent ?? null }),
    }
  );
}

// SSH Connection Command Info (Phase 6 - P6-T7)
export async function fetchSshConnectionInfo(
  query?: import('../types/api').SshConnectionInfoQuery
): Promise<import('../types/api').SshConnectionInfoResponse> {
  const params = new URLSearchParams();
  if (query?.site_id !== undefined) params.set('site_id', query.site_id.toString());
  if (query?.path !== undefined) params.set('path', query.path);
  if (query?.tenant !== undefined) params.set('tenant', query.tenant);
  if (query?.port !== undefined) params.set('port', query.port.toString());
  if (query?.host !== undefined) params.set('host', query.host);

  const qs = params.toString() ? `?${params.toString()}` : '';
  return request<import('../types/api').SshConnectionInfoResponse>(
    `/api/v1/ssh/connection-info${qs}`
  );
}

// ==========================================
// Node.js Runtime API (Phase 7 - P7-T1)
// ==========================================

export async function fetchNodeVersions(
  mirrorUrl?: string,
  refresh?: boolean
): Promise<import('../types/api').NodeRuntimeListApiResponse> {
  const params = new URLSearchParams();
  if (mirrorUrl) params.set('mirror_url', mirrorUrl);
  if (refresh) params.set('refresh', 'true');
  const qs = params.toString() ? `?${params.toString()}` : '';
  return request<import('../types/api').NodeRuntimeListApiResponse>(
    `/api/v1/runtimes/node/versions${qs}`
  );
}

export async function installNodeVersion(
  payload: import('../types/api').NodeRuntimeInstallRequest
): Promise<JobItem> {
  return request<JobItem>(
    '/api/v1/runtimes/node/install',
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function removeNodeVersion(
  version: string
): Promise<import('../types/api').NodeRuntimeActionResponse> {
  return request<import('../types/api').NodeRuntimeActionResponse>(
    `/api/v1/runtimes/node/${encodeURIComponent(version)}`,
    {
      method: 'DELETE',
    }
  );
}

export async function enableNodeCorepack(
  version: string
): Promise<import('../types/api').NodeRuntimeActionResponse> {
  return request<import('../types/api').NodeRuntimeActionResponse>(
    `/api/v1/runtimes/node/${encodeURIComponent(version)}/corepack`,
    {
      method: 'POST',
    }
  );
}

// Node.js Site & Process Management Client APIs (Phase 7 - P7-T2)
export async function getNodeSiteStatus(
  siteId: number
): Promise<import('../types/api').NodeSiteStatusApiResponse> {
  return request<import('../types/api').NodeSiteStatusApiResponse>(
    `/api/v1/sites/${siteId}/node/status`
  );
}

export async function postNodeSiteAction(
  siteId: number,
  payload: import('../types/api').NodeSiteActionApiRequest
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>(
    `/api/v1/sites/${siteId}/node/action`,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function runNodeSiteJob(
  siteId: number,
  payload: import('../types/api').NodeSiteJobApiRequest
): Promise<import('../types/api').NodeSiteJobApiResponse> {
  return request<import('../types/api').NodeSiteJobApiResponse>(
    `/api/v1/sites/${siteId}/node/job`,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function getNodeSiteSettings(
  siteId: number
): Promise<import('../types/api').NodeSiteSettingsDto> {
  return request<import('../types/api').NodeSiteSettingsDto>(
    `/api/v1/sites/${siteId}/node/settings`
  );
}

export async function updateNodeSiteSettings(
  siteId: number,
  payload: import('../types/api').UpdateNodeSiteSettingsRequest
): Promise<import('../types/api').NodeSiteSettingsDto> {
  return request<import('../types/api').NodeSiteSettingsDto>(
    `/api/v1/sites/${siteId}/node/settings`,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    }
  );
}

// Python Runtime & Site Management Client APIs (Phase 7 - P7-T3)
export interface RuntimeCatalogStatus {
  enabled: boolean;
  interval_hours: number;
  sources: { runtime: string; checked_at: number | null; refreshed_at: number | null; error: string | null; stale: boolean }[];
}
export function getRuntimeCatalog(): Promise<RuntimeCatalogStatus> {
  return request<RuntimeCatalogStatus>('/api/v1/runtimes/catalog');
}
export function saveRuntimeCatalogSchedule(payload: { enabled: boolean; interval_hours: number }): Promise<RuntimeCatalogStatus> {
  return request<RuntimeCatalogStatus>('/api/v1/runtimes/catalog', { method: 'PUT', body: JSON.stringify(payload) });
}
export function refreshRuntimeCatalog(): Promise<RuntimeCatalogStatus> {
  return request<RuntimeCatalogStatus>('/api/v1/runtimes/catalog/refresh', { method: 'POST' });
}
export function fetchPythonCatalog(): Promise<import('../types/api').PythonRuntimeListApiResponse> {
  return request<import('../types/api').PythonRuntimeListApiResponse>('/api/v1/runtimes/python/versions');
}
export async function getPythonVersions(
  refresh?: boolean
): Promise<import('../types/api').PythonVersionSummaryDto[]> {
  const qs = refresh ? '?refresh=true' : '';
  const res = await request<import('../types/api').PythonRuntimeListApiResponse>(
    `/api/v1/runtimes/python${qs}`
  );
  return res.versions;
}

export async function installPythonVersion(
  payload: { version: string }
): Promise<import('../types/api').JobItem> {
  return request<import('../types/api').JobItem>(
    '/api/v1/runtimes/python/install',
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function removePythonVersion(
  version: string
): Promise<{ success: boolean; message: string; version: string }> {
  return request<{ success: boolean; message: string; version: string }>(
    `/api/v1/runtimes/python/${encodeURIComponent(version)}`,
    {
      method: 'DELETE',
    }
  );
}

export async function getPythonSiteStatus(
  siteId: number
): Promise<import('../types/api').PythonSiteStatusApiResponse> {
  return request<import('../types/api').PythonSiteStatusApiResponse>(
    `/api/v1/sites/${siteId}/python/status`
  );
}

export async function postPythonSiteAction(
  siteId: number,
  payload: import('../types/api').PythonSiteActionApiRequest
): Promise<{ success: boolean; message: string }> {
  return request<{ success: boolean; message: string }>(
    `/api/v1/sites/${siteId}/python/action`,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function runPythonSiteJob(
  siteId: number,
  payload: import('../types/api').PythonSiteJobApiRequest
): Promise<import('../types/api').PythonSiteJobApiResponse> {
  return request<import('../types/api').PythonSiteJobApiResponse>(
    `/api/v1/sites/${siteId}/python/job`,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function getPythonSiteSettings(
  siteId: number
): Promise<import('../types/api').PythonSiteSettingsDto> {
  return request<import('../types/api').PythonSiteSettingsDto>(
    `/api/v1/sites/${siteId}/python/settings`
  );
}

export async function updatePythonSiteSettings(
  siteId: number,
  payload: import('../types/api').UpdatePythonSiteSettingsRequest
): Promise<import('../types/api').PythonSiteSettingsDto> {
  return request<import('../types/api').PythonSiteSettingsDto>(
    `/api/v1/sites/${siteId}/python/settings`,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    }
  );
}

// Reverse Proxy API (Phase 7 - P7-T4)
export async function getProxySiteSettings(
  siteId: number
): Promise<import('../types/api').ProxySiteSettingsDto> {
  return request<import('../types/api').ProxySiteSettingsDto>(
    `/api/v1/sites/${siteId}/proxy/settings`
  );
}

export async function updateProxySiteSettings(
  siteId: number,
  payload: import('../types/api').UpdateProxySiteSettingsRequest
): Promise<import('../types/api').ProxySiteSettingsDto> {
  return request<import('../types/api').ProxySiteSettingsDto>(
    `/api/v1/sites/${siteId}/proxy/settings`,
    {
      method: 'PUT',
      body: JSON.stringify(payload),
    }
  );
}

export async function probeProxyUpstream(
  siteId: number,
  upstreamUrl?: string
): Promise<import('../types/api').ProxyUpstreamProbeResponse> {
  const url = upstreamUrl
    ? `/api/v1/sites/${siteId}/proxy/probe?upstream_url=${encodeURIComponent(upstreamUrl)}`
    : `/api/v1/sites/${siteId}/proxy/probe`;
  return request<import('../types/api').ProxyUpstreamProbeResponse>(url);
}

// Unified Process Management API (Phase 7 - P7-T5)
export async function fetchProcesses(): Promise<import('../types/api').ProcessListResponse> {
  return request<import('../types/api').ProcessListResponse>('/api/v1/processes');
}

// Environment Variables API (Phase 8 - P8-T1)
export async function fetchSiteEnv(
  siteId: number,
  reveal = false
): Promise<import('../types/api').SiteEnvResponse> {
  const url = reveal
    ? `/api/v1/sites/${siteId}/env?reveal=true`
    : `/api/v1/sites/${siteId}/env`;
  return request<import('../types/api').SiteEnvResponse>(url);
}

export async function updateSiteEnv(
  siteId: number,
  req: import('../types/api').UpdateSiteEnvRequest
): Promise<import('../types/api').SiteEnvActionResponse> {
  return request<import('../types/api').SiteEnvActionResponse>(
    `/api/v1/sites/${siteId}/env`,
    {
      method: 'PUT',
      body: JSON.stringify(req),
    }
  );
}

export async function importSiteDotEnv(
  siteId: number,
  req: import('../types/api').ImportDotEnvRequest
): Promise<import('../types/api').SiteEnvActionResponse> {
  return request<import('../types/api').SiteEnvActionResponse>(
    `/api/v1/sites/${siteId}/env/import`,
    {
      method: 'POST',
      body: JSON.stringify(req),
    }
  );
}

export async function restartSiteForEnv(
  siteId: number
): Promise<import('../types/api').SiteEnvActionResponse> {
  return request<import('../types/api').SiteEnvActionResponse>(
    `/api/v1/sites/${siteId}/env/restart`,
    {
      method: 'POST',
    }
  );
}

// Cron Job Management API (Phase 8 - P8-T2)
export async function fetchSiteCronJobs(
  siteId: number
): Promise<import('../types/api').CronJobListResponse> {
  return request<import('../types/api').CronJobListResponse>(
    `/api/v1/sites/${siteId}/cron`
  );
}

export async function createSiteCronJob(
  siteId: number,
  req: import('../types/api').CreateCronJobRequest
): Promise<import('../types/api').CronJob> {
  return request<import('../types/api').CronJob>(
    `/api/v1/sites/${siteId}/cron`,
    {
      method: 'POST',
      body: JSON.stringify(req),
    }
  );
}

export async function getSiteCronJob(
  siteId: number,
  jobId: number
): Promise<import('../types/api').CronJob> {
  return request<import('../types/api').CronJob>(
    `/api/v1/sites/${siteId}/cron/${jobId}`
  );
}

export async function updateSiteCronJob(
  siteId: number,
  jobId: number,
  req: import('../types/api').UpdateCronJobRequest
): Promise<import('../types/api').CronJob> {
  return request<import('../types/api').CronJob>(
    `/api/v1/sites/${siteId}/cron/${jobId}`,
    {
      method: 'PUT',
      body: JSON.stringify(req),
    }
  );
}

export async function deleteSiteCronJob(
  siteId: number,
  jobId: number
): Promise<{ message: string }> {
  return request<{ message: string }>(
    `/api/v1/sites/${siteId}/cron/${jobId}`,
    {
      method: 'DELETE',
    }
  );
}

export async function runSiteCronJob(
  siteId: number,
  jobId: number
): Promise<import('../types/api').CronRun> {
  return request<import('../types/api').CronRun>(
    `/api/v1/sites/${siteId}/cron/${jobId}/run`,
    {
      method: 'POST',
    }
  );
}

export async function fetchSiteCronRuns(
  siteId: number,
  jobId: number,
  limit?: number
): Promise<import('../types/api').CronRun[]> {
  const q = limit ? `?limit=${limit}` : '';
  return request<import('../types/api').CronRun[]>(
    `/api/v1/sites/${siteId}/cron/${jobId}/runs${q}`
  );
}

export async function fetchSiteCronLogs(
  siteId: number,
  jobId: number,
  lines?: number
): Promise<{ logs: string[] }> {
  const q = lines ? `?lines=${lines}` : '';
  return request<{ logs: string[] }>(
    `/api/v1/sites/${siteId}/cron/${jobId}/logs${q}`
  );
}

export async function validateCronExpression(
  expression: string
): Promise<import('../types/api').ValidateCronResponse> {
  return request<import('../types/api').ValidateCronResponse>(
    `/api/v1/sites/0/cron/validate`,
    {
      method: 'POST',
      body: JSON.stringify({ expression }),
    }
  );
}

// Site Rules API (Phase 8 - P8-T3)

export async function fetchSiteRules(siteId: number): Promise<SiteRulesConfig> {
  return request<SiteRulesConfig>(`/api/v1/sites/${siteId}/rules`);
}

export async function updateSiteRules(
  siteId: number,
  rules: SiteRulesConfig
): Promise<SiteRulesConfig> {
  return request<SiteRulesConfig>(`/api/v1/sites/${siteId}/rules`, {
    method: 'PUT',
    body: JSON.stringify(rules),
  });
}

export async function hashBasicAuthPassword(
  siteId: number,
  password: string
): Promise<HashPasswordResponse> {
  return request<HashPasswordResponse>(
    `/api/v1/sites/${siteId}/rules/hash-password`,
    {
      method: 'POST',
      body: JSON.stringify({ password }),
    }
  );
}

export async function validateNginxSnippet(
  siteId: number,
  content: string
): Promise<ValidateSnippetResponse> {
  return request<ValidateSnippetResponse>(
    `/api/v1/sites/${siteId}/rules/validate-snippet`,
    {
      method: 'POST',
      body: JSON.stringify({ content }),
    }
  );
}

// Git Deploy & Webhook API (Phase 8 - P8-T4)

export async function fetchSiteGitConfig(
  siteId: number
): Promise<SiteGitConfig> {
  return request<SiteGitConfig>(`/api/v1/sites/${siteId}/git`);
}

export async function updateSiteGitConfig(
  siteId: number,
  config: UpsertGitConfigRequest
): Promise<SiteGitConfig> {
  return request<SiteGitConfig>(`/api/v1/sites/${siteId}/git`, {
    method: 'PUT',
    body: JSON.stringify(config),
  });
}

export async function generateSiteDeployKey(
  siteId: number
): Promise<GenerateDeployKeyResponse> {
  return request<GenerateDeployKeyResponse>(
    `/api/v1/sites/${siteId}/git/key/generate`,
    {
      method: 'POST',
    }
  );
}

export async function triggerSiteDeploy(
  siteId: number,
  payload?: TriggerDeployRequest
): Promise<SiteDeployment> {
  return request<SiteDeployment>(`/api/v1/sites/${siteId}/git/deploy`, {
    method: 'POST',
    body: payload ? JSON.stringify(payload) : JSON.stringify({}),
  });
}

export async function rollbackSiteDeploy(
  siteId: number,
  targetReleaseId: string
): Promise<SiteDeployment> {
  const payload: RollbackDeployRequest = { target_release_id: targetReleaseId };
  return request<SiteDeployment>(`/api/v1/sites/${siteId}/git/rollback`, {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function fetchSiteDeployments(
  siteId: number,
  limit?: number
): Promise<SiteDeployment[]> {
  const query = limit ? `?limit=${limit}` : '';
  return request<SiteDeployment[]>(
    `/api/v1/sites/${siteId}/git/deployments${query}`
  );
}

export async function fetchSiteDeploymentDetail(
  siteId: number,
  deployId: number
): Promise<SiteDeployment> {
  return request<SiteDeployment>(
    `/api/v1/sites/${siteId}/git/deployments/${deployId}`
  );
}

// ==========================================
// Database Provisioner API (Phase 8 - P8-T5)
// ==========================================

export async function fetchDatabaseServers(): Promise<DatabaseServer[]> {
  return request<DatabaseServer[]>('/api/v1/databases/servers');
}

export async function fetchAvailableDatabaseEngines(): Promise<import('../types/api').AvailableDatabaseEngine[]> {
  const data = await request<unknown>('/api/v1/databases/engines');
  if (!Array.isArray(data) || data.some(item => !item || typeof item !== 'object'
    || typeof item.engine !== 'string' || !/^[a-z][a-z0-9_]*$/.test(item.engine)
    || typeof item.label !== 'string' || !item.label.trim()
    || !Number.isInteger(item.port) || item.port < 1 || item.port > 65535)
    || new Set(data.map(item => item.engine)).size !== data.length) {
    throw new Error('Format pilihan mesin basis data tidak valid. Muat ulang lalu coba lagi.');
  }
  return data;
}

export async function fetchDatabaseWebTools(): Promise<{ installed: boolean; ready: boolean }> {
  const data = await request<unknown>('/api/v1/databases/web-tools');
  if (!data || typeof data !== 'object' || !('installed' in data) || !('ready' in data)
    || typeof data.installed !== 'boolean' || typeof data.ready !== 'boolean') {
    throw new Error('Format status pengelola database tidak valid. Perbarui panel lalu coba lagi.');
  }
  return { installed: data.installed, ready: data.ready };
}

export async function installDatabaseWebTools(): Promise<JobItem> {
  return request('/api/v1/databases/web-tools/install', { method: 'POST' });
}

export async function openDatabaseWebAccess(id: number): Promise<{ url: string; tool: string }> {
  return request(`/api/v1/databases/${id}/web-access`, { method: 'POST' });
}

export async function installDatabaseServer(
  engine: DatabaseEngine
): Promise<DatabaseActionResponse> {
  return request<DatabaseActionResponse>('/api/v1/databases/servers/install', {
    method: 'POST',
    body: JSON.stringify({ engine }),
  });
}

export async function actionDatabaseServer(
  engine: DatabaseEngine,
  action: 'start' | 'stop' | 'restart'
): Promise<DatabaseActionResponse> {
  return request<DatabaseActionResponse>('/api/v1/databases/servers/action', {
    method: 'POST',
    body: JSON.stringify({ engine, action }),
  });
}

export async function fetchDatabases(params?: {
  site_id?: number;
  user_id?: number;
}): Promise<DatabaseListResponse> {
  const queryParts: string[] = [];
  if (params?.site_id !== undefined) {
    queryParts.push(`site_id=${params.site_id}`);
  }
  if (params?.user_id !== undefined) {
    queryParts.push(`user_id=${params.user_id}`);
  }
  const query = queryParts.length > 0 ? `?${queryParts.join('&')}` : '';
  return parseDatabaseList(await request<unknown>(`/api/v1/databases${query}`));
}

export async function fetchDatabase(id: number): Promise<DatabaseItem> {
  return request<DatabaseItem>(`/api/v1/databases/${id}`);
}

export async function createDatabase(
  payload: CreateDatabaseRequest
): Promise<CreateDatabaseResponse> {
  return request<CreateDatabaseResponse>('/api/v1/databases', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function deleteDatabase(id: number): Promise<DatabaseActionResponse> {
  return request<DatabaseActionResponse>(`/api/v1/databases/${id}`, {
    method: 'DELETE',
  });
}

export async function rotateDatabasePassword(
  id: number,
  payload?: RotatePasswordRequest
): Promise<RotatePasswordResponse> {
  return request<RotatePasswordResponse>(`/api/v1/databases/${id}/rotate-password`, {
    method: 'POST',
    body: payload ? JSON.stringify(payload) : JSON.stringify({}),
  });
}

export async function updateDatabaseRemoteAccess(
  id: number,
  payload: UpdateRemoteAccessRequest
): Promise<DatabaseItem> {
  return request<DatabaseItem>(`/api/v1/databases/${id}/remote-access`, {
    method: 'PUT',
    body: JSON.stringify(payload),
  });
}

export async function exportDatabase(
  id: number,
  payload?: ExportDatabaseRequest
): Promise<DatabaseActionResponse> {
  return request<DatabaseActionResponse>(`/api/v1/databases/${id}/export`, {
    method: 'POST',
    body: payload ? JSON.stringify(payload) : JSON.stringify({}),
  });
}

export async function importDatabase(
  id: number,
  payload: ImportDatabaseRequest
): Promise<DatabaseActionResponse> {
  return request<DatabaseActionResponse>(`/api/v1/databases/${id}/import`, {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

// Site Backup & Snapshot Restore API (Phase 8 - P8-T6)

export async function fetchSiteBackups(
  siteId: number
): Promise<BackupListResponse> {
  return request<BackupListResponse>(`/api/v1/sites/${siteId}/backups`);
}

export async function createSiteBackup(
  siteId: number,
  payload: CreateBackupRequest
): Promise<{ message: string; job_id: string; backup_id: number }> {
  return request<{ message: string; job_id: string; backup_id: number }>(
    `/api/v1/sites/${siteId}/backups`,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function getSiteBackup(
  siteId: number,
  backupId: number
): Promise<BackupItem> {
  return request<BackupItem>(`/api/v1/sites/${siteId}/backups/${backupId}`);
}

export async function deleteSiteBackup(
  siteId: number,
  backupId: number
): Promise<{ message: string }> {
  return request<{ message: string }>(
    `/api/v1/sites/${siteId}/backups/${backupId}`,
    {
      method: 'DELETE',
    }
  );
}

export async function restoreSiteBackup(
  siteId: number,
  backupId: number,
  payload: RestoreBackupRequest
): Promise<{ message: string; job_id: string }> {
  return request<{ message: string; job_id: string }>(
    `/api/v1/sites/${siteId}/backups/${backupId}/restore`,
    {
      method: 'POST',
      body: JSON.stringify(payload),
    }
  );
}

export async function fetchBackupSchedule(
  siteId: number
): Promise<BackupSchedule | null> {
  return request<BackupSchedule | null>(`/api/v1/sites/${siteId}/backups/schedule`);
}

export async function updateBackupSchedule(
  siteId: number,
  payload: UpdateScheduleRequest
): Promise<BackupSchedule> {
  return request<BackupSchedule>(`/api/v1/sites/${siteId}/backups/schedule`, {
    method: 'PUT',
    body: JSON.stringify(payload),
  });
}

export function getBackupDownloadUrl(siteId: number, backupId: number): string {
  return `/api/v1/sites/${siteId}/backups/${backupId}/download`;
}

// ---------------------------------------------------------------------------
// System Notifications API
// ---------------------------------------------------------------------------

export async function fetchNotifications(params?: {
  page?: number;
  per_page?: number;
  unread_only?: boolean;
  category?: string;
  severity?: string;
}): Promise<ListNotificationsResponse> {
  const query = new URLSearchParams();
  if (params?.page) query.set('page', params.page.toString());
  if (params?.per_page) query.set('per_page', params.per_page.toString());
  if (params?.unread_only !== undefined) query.set('unread_only', params.unread_only.toString());
  if (params?.category) query.set('category', params.category);
  if (params?.severity) query.set('severity', params.severity);

  const qs = query.toString();
  return request<ListNotificationsResponse>(`/api/v1/notifications${qs ? `?${qs}` : ''}`);
}

export async function fetchUnreadNotificationCount(): Promise<{ unread_count: number }> {
  return request<{ unread_count: number }>('/api/v1/notifications/unread-count');
}

export async function markNotificationRead(id: string): Promise<{ success: boolean; id: string }> {
  return request<{ success: boolean; id: string }>(`/api/v1/notifications/${id}/read`, {
    method: 'PATCH',
  });
}

export async function markAllNotificationsRead(): Promise<{ success: boolean; marked_count: number }> {
  return request<{ success: boolean; marked_count: number }>('/api/v1/notifications/mark-all-read', {
    method: 'POST',
  });
}

export async function deleteNotification(id: string): Promise<{ success: boolean; id: string }> {
  return request<{ success: boolean; id: string }>(`/api/v1/notifications/${id}`, {
    method: 'DELETE',
  });
}

export async function clearNotifications(readOnly = false): Promise<{ cleared_count: number }> {
  const qs = readOnly ? '?read_only=true' : '';
  return request<{ cleared_count: number }>(`/api/v1/notifications/clear${qs}`, {
    method: 'POST',
  });
}

export async function fetchNotificationSettings(): Promise<NotificationSettings> {
  return request<NotificationSettings>('/api/v1/notifications/settings');
}

export async function updateNotificationSettings(
  payload: UpdateNotificationSettingsRequest
): Promise<NotificationSettings> {
  return request<NotificationSettings>('/api/v1/notifications/settings', {
    method: 'PUT',
    body: JSON.stringify(payload),
  });
}

export async function testSmtpConnection(payload: TestSmtpRequest): Promise<TestSmtpResponse> {
  return request<TestSmtpResponse>('/api/v1/notifications/test-smtp', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

export async function evaluateNotificationAlerts(): Promise<EvaluateAlertsResponse> {
  return request<EvaluateAlertsResponse>('/api/v1/notifications/evaluate-alerts', {
    method: 'POST',
  });
}

// ---------------------------------------------------------------------------
// Analytics & Timeseries API (Phase 9 - P9-T5 & P9-T6)
// ---------------------------------------------------------------------------

export async function fetchSiteAnalyticsOverview(
  siteId: number,
  params?: AnalyticsTimeRangeQuery
): Promise<SiteAnalyticsOverviewResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.interval) query.set('interval', params.interval);
  if (params?.compare !== undefined) query.set('compare', params.compare.toString());
  if (params?.prev_from !== undefined) query.set('prev_from', params.prev_from.toString());
  if (params?.prev_to !== undefined) query.set('prev_to', params.prev_to.toString());

  const qs = query.toString();
  return request<SiteAnalyticsOverviewResponse>(`/api/v1/sites/${siteId}/analytics/overview${qs ? `?${qs}` : ''}`);
}

export async function fetchSiteAnalyticsTimeseries(
  siteId: number,
  params?: AnalyticsTimeRangeQuery
): Promise<SiteTimeseriesResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.interval) query.set('interval', params.interval);

  const qs = query.toString();
  return request<SiteTimeseriesResponse>(`/api/v1/sites/${siteId}/analytics/timeseries${qs ? `?${qs}` : ''}`);
}

export async function fetchSiteAnalyticsBreakdowns(
  siteId: number,
  params?: AnalyticsBreakdownsQuery
): Promise<SiteBreakdownsResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.dimension) query.set('dimension', params.dimension);
  if (params?.limit !== undefined) query.set('limit', params.limit.toString());

  const qs = query.toString();
  return request<SiteBreakdownsResponse>(`/api/v1/sites/${siteId}/analytics/breakdowns${qs ? `?${qs}` : ''}`);
}

export async function fetchSiteAppMetrics(
  siteId: number,
  params?: AnalyticsTimeRangeQuery
): Promise<SiteAppMetricsResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());

  const qs = query.toString();
  return request<SiteAppMetricsResponse>(`/api/v1/sites/${siteId}/analytics/app${qs ? `?${qs}` : ''}`);
}

export async function fetchSiteSlowlog(
  siteId: number,
  params?: AnalyticsSlowlogQuery
): Promise<SiteSlowlogResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.limit !== undefined) query.set('limit', params.limit.toString());

  const qs = query.toString();
  return request<SiteSlowlogResponse>(`/api/v1/sites/${siteId}/analytics/slowlog${qs ? `?${qs}` : ''}`);
}

export function getSiteAnalyticsExportUrl(siteId: number, params?: AnalyticsExportQuery): string {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.interval) query.set('interval', params.interval);

  const qs = query.toString();
  return `/api/v1/sites/${siteId}/analytics/export${qs ? `?${qs}` : ''}`;
}

export async function fetchSystemAnalyticsTimeseries(
  params?: AnalyticsTimeRangeQuery
): Promise<SystemTimeseriesResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.interval) query.set('interval', params.interval);

  const qs = query.toString();
  return parseSystemTimeseries(await request<unknown>(`/api/v1/system/analytics/timeseries${qs ? `?${qs}` : ''}`));
}

// Analytics Insights Engine (Phase 9 - P9-T7)
export async function fetchSiteInsights(
  siteId: number
): Promise<SiteInsightsResponse> {
  return request<SiteInsightsResponse>(`/api/v1/sites/${siteId}/analytics/insights`);
}

export async function evaluateSiteInsights(
  siteId: number
): Promise<SiteInsightsResponse> {
  return request<SiteInsightsResponse>(`/api/v1/sites/${siteId}/analytics/insights/evaluate`, {
    method: 'POST',
  });
}

export async function fetchSystemInsights(): Promise<SystemInsightsResponse> {
  return parseSystemInsights(await request<unknown>('/api/v1/system/analytics/insights'));
}

// Security Analytics & IP Banning (Phase 9 - P9-T8)
export async function fetchSiteSecurityOverview(
  siteId: number,
  params?: { from?: number; to?: number; limit?: number }
): Promise<SecurityOverviewResponse> {
  const query = new URLSearchParams();
  if (params?.from !== undefined) query.set('from', params.from.toString());
  if (params?.to !== undefined) query.set('to', params.to.toString());
  if (params?.limit !== undefined) query.set('limit', params.limit.toString());

  const qs = query.toString();
  return request<SecurityOverviewResponse>(
    `/api/v1/sites/${siteId}/analytics/security/overview${qs ? `?${qs}` : ''}`
  );
}

export async function fetchSiteBannedIps(
  siteId: number
): Promise<BannedIpDto[]> {
  return request<BannedIpDto[]>(`/api/v1/sites/${siteId}/security/banned-ips`);
}

export async function banSiteIp(
  siteId: number,
  data: BanIpRequest
): Promise<BannedIpDto> {
  return request<BannedIpDto>(`/api/v1/sites/${siteId}/security/ban-ip`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function unbanSiteIp(
  siteId: number,
  ip: string
): Promise<UnbanIpResponse> {
  return request<UnbanIpResponse>(
    `/api/v1/sites/${siteId}/security/ban-ip/${encodeURIComponent(ip)}`,
    {
      method: 'DELETE',
    }
  );
}

// Data Lifecycle & Retention Management (Phase 9 - P9-T9)
export async function fetchAnalyticsRetention(): Promise<AnalyticsRetentionResponse> {
  return parseRetention(await request<unknown>('/api/v1/system/analytics/retention'));
}

export async function updateAnalyticsRetention(
  data: UpdateRetentionRequest
): Promise<AnalyticsRetentionResponse> {
  return parseRetention(await request<unknown>('/api/v1/system/analytics/retention', {
    method: 'PUT',
    body: JSON.stringify(retentionUpdatePayload(data)),
  }));
}

export async function triggerAnalyticsMaintenance(
  data: TriggerMaintenanceRequest = {}
): Promise<RetentionMaintenanceReportDto> {
  return parseMaintenance(await request<unknown>('/api/v1/system/analytics/maintenance', {
    method: 'POST',
    body: JSON.stringify({ force_vacuum: data.force_vacuum }),
  }));
}

export async function checkAnalyticsIntegrity(): Promise<IntegrityCheckReportDto> {
  return parseIntegrity(await request<unknown>('/api/v1/system/analytics/integrity-check', {
    method: 'POST',
  }));
}

// Server Optimizer & Hardware Tuning (Phase 10 - P10-T2)
export async function fetchOptimizerStatus(demand?: PackageDemand): Promise<OptimizerStatusResponseDto> {
  const query = demand ? `?${new URLSearchParams({
    capacity_usage: demand.usage,
    projects_per_tenant: String(demand.projects_per_tenant),
    tenant_slots: String(demand.tenant_slots),
  })}` : '';
  const status = await request<OptimizerStatusResponseDto>(`/api/v1/system/optimizer${query}`);
  if (demand && !status.package_preset) {
    throw new Error('Pratinjau kapasitas belum didukung API ini. Perbarui paket Qinpanel lengkap.');
  }
  return status;
}

export async function applyOptimizerProfile(
  data: ApplyOptimizerRequestDto
): Promise<OptimizerActionResponseDto> {
  return request<OptimizerActionResponseDto>('/api/v1/system/optimizer/apply', {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function rollbackOptimizerProfile(): Promise<OptimizerActionResponseDto> {
  return request<OptimizerActionResponseDto>('/api/v1/system/optimizer/rollback', {
    method: 'POST',
  });
}

// Memory Profiling & PRD Section 9 RAM Validation (Phase 10 - P10-T3)
export async function fetchMemoryProfile(): Promise<PanelMemoryProfileReport> {
  return request<PanelMemoryProfileReport>('/api/v1/system/memory-profile');
}

// Security Hardening & OWASP ASVS L2 / nftables (Phase 10 - P10-T4)
export async function fetchSecurityHardeningOverview(): Promise<SecurityHardeningOverviewResponse> {
  return request<SecurityHardeningOverviewResponse>('/api/v1/system/security-hardening');
}

export async function syncNftablesLoopback(
  payload: NftablesSyncPayload = {}
): Promise<NftablesSyncResponseDto> {
  return request<NftablesSyncResponseDto>('/api/v1/system/security-hardening/nftables/sync', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}

// System Reliability, Doctor, Reconcile, & Backups (Phase 10 - P10-T5)
export async function fetchSystemDoctorReport(): Promise<DoctorReport> {
  return request<DoctorReport>('/api/v1/system/doctor');
}

export async function fixSystemDoctor(): Promise<DoctorReport> {
  return request<DoctorReport>('/api/v1/system/doctor/fix', {
    method: 'POST',
  });
}

export async function fetchSystemReconcileReport(): Promise<ReconcileReport> {
  return request<ReconcileReport>('/api/v1/system/reconcile');
}

export async function applySystemReconcile(): Promise<ReconcileReport> {
  return request<ReconcileReport>('/api/v1/system/reconcile', {
    method: 'POST',
  });
}

export async function fetchSystemBackups(): Promise<SystemBackupSummary[]> {
  return request<SystemBackupSummary[]>('/api/v1/system/backups');
}

export async function createSystemBackup(
  payload?: CreateSystemBackupPayload
): Promise<SystemBackupSummary> {
  return request<SystemBackupSummary>('/api/v1/system/backups', {
    method: 'POST',
    body: JSON.stringify(payload || {}),
  });
}

export async function restoreSystemBackup(
  payload: RestoreSystemBackupPayload
): Promise<SystemBackupVerificationResult> {
  return request<SystemBackupVerificationResult>('/api/v1/system/backups/restore', {
    method: 'POST',
    body: JSON.stringify(payload),
  });
}
