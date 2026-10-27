export interface HealthResponse {
  status: string;
  timestamp: number;
}

export interface SystemInfoResponse {
  api_version: string;
  agent_version: string | null;
  proto_version: number;
  os: string;
  arch: string;
  agent_connected: boolean;
  environment: string;
}

export type ThemeMode = 'light' | 'dark' | 'system';

export interface User {
  id: number;
  username: string;
  role: 'admin' | 'user' | 'reseller' | string;
  email: string | null;
  totp_enabled: boolean;
  impersonated_by?: string | null;
}

export interface LoginResponse {
  message?: string;
  user?: User;
  requires_2fa?: boolean;
  csrf_token?: string;
}

export interface CsrfResponse {
  csrf_token: string;
}

export interface TwoFactorSetupResponse {
  secret: string;
  uri: string;
  qr_code_base64: string;
}

export interface TwoFactorStatusResponse {
  enabled: boolean;
  remaining_recovery_codes: number;
}

export interface TwoFactorEnableResponse {
  message: string;
  recovery_codes: string[];
}

export type AuditCategory = 'authentication' | 'users' | 'websites' | 'files' | 'databases' | 'runtimes' | 'access' | 'network' | 'email' | 'backups' | 'system' | 'other';

export interface AuditLogItem {
  category: AuditCategory;
  id: number;
  user_id: number | null;
  actor_username: string;
  action: string;
  target_resource: string;
  details_json: string | null;
  ip_address: string | null;
  status: string;
  created_at: number;
}

export interface AuditLogListResponse {
  items: AuditLogItem[];
  total: number;
  page: number;
  per_page: number;
  total_pages: number;
}

export interface AuditLogQuery {
  category?: AuditCategory;
  q?: string;
  page?: number;
  per_page?: number;
  user_id?: number;
  actor?: string;
  action?: string;
  target?: string;
  status?: string;
  from?: number;
  to?: number;
}

export type JobStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled';

export interface JobItem {
  id: string;
  user_id: number | null;
  name: string;
  status: JobStatus;
  progress: number;
  payload_json: string;
  result_json: string | null;
  error_message: string | null;
  retry_count: number;
  max_retries: number;
  created_at: number;
  updated_at: number;
}

export interface JobListResponse {
  items: JobItem[];
  total: number;
  page: number;
  per_page: number;
  total_pages: number;
}

export interface JobProgressEvent {
  sequence?: number;
  job_id: string;
  status: JobStatus;
  progress: number;
  message: string | null;
  timestamp: number;
}

export type ShellPolicy = 'none' | 'sftp' | 'full';

export interface Plan {
  id: number;
  name: string;
  description: string | null;
  cpu_quota_percent: number;
  memory_limit_bytes: number;
  memory_high_bytes: number | null;
  tasks_max: number;
  io_weight: number;
  disk_limit_bytes: number;
  inode_limit: number;
  max_sites: number;
  max_domains: number;
  max_databases: number;
  max_cron_jobs: number;
  max_terminal_sessions: number;
  max_upload_bytes: number;
  shell_policy: ShellPolicy;
  allow_external_upstream: boolean;
  created_at: number;
  updated_at: number;
}

export interface CreatePlanRequest {
  name: string;
  description?: string;
  cpu_quota_percent?: number;
  memory_limit_bytes?: number;
  memory_high_bytes?: number;
  tasks_max?: number;
  io_weight?: number;
  disk_limit_bytes?: number;
  inode_limit?: number;
  max_sites?: number;
  max_domains?: number;
  max_databases?: number;
  max_cron_jobs?: number;
  max_terminal_sessions?: number;
  max_upload_bytes?: number;
  shell_policy?: ShellPolicy;
  allow_external_upstream?: boolean;
}

export interface UpdatePlanRequest {
  name?: string;
  description?: string;
  cpu_quota_percent?: number;
  memory_limit_bytes?: number;
  memory_high_bytes?: number;
  tasks_max?: number;
  io_weight?: number;
  disk_limit_bytes?: number;
  inode_limit?: number;
  max_sites?: number;
  max_domains?: number;
  max_databases?: number;
  max_cron_jobs?: number;
  max_terminal_sessions?: number;
  max_upload_bytes?: number;
  shell_policy?: ShellPolicy;
  allow_external_upstream?: boolean;
}

export interface PlanListResponse {
  plans: Plan[];
}

export interface PlanResponse {
  plan: Plan;
}

export interface TenantListItem {
  id: number;
  username: string;
  email: string | null;
  status: string;
  linux_uid: number | null;
  linux_username: string | null;
  plan_id: number | null;
  plan_name: string | null;
  shell_policy?: string | null;
  created_at: number;
  updated_at: number;
}

export interface TenantListResponse {
  tenants: TenantListItem[];
}

export interface TenantDetailResponse {
  tenant: TenantListItem;
  plan: Plan | null;
}

export interface CreateTenantRequest {
  username_slug: string;
  email?: string;
  password: string;
  plan_id: number;
}

export interface SliceResourceMetrics {
  uid: number;
  slice_name: string;
  exists: boolean;
  cpu_usage_usec: number;
  memory_current_bytes: number;
  memory_peak_bytes?: number | null;
  pids_current: number;
  pids_peak?: number | null;
  io_read_bytes: number;
  io_write_bytes: number;
}

export interface QuotaMetrics {
  uid: number;
  username: string;
  bytes_used: number;
  bytes_limit: number;
  inodes_used: number;
  inodes_limit: number;
  backend: string;
}

export interface TenantResourceUsage {
  uid: number;
  linux_username: string;
  metrics: SliceResourceMetrics;
  quota?: QuotaMetrics | null;
}

export interface PrerequisiteCheckItem {
  name: string;
  ok: boolean;
  message: string;
  details?: string | null;
}

export interface HardeningStatus {
  cgroup_v2: PrerequisiteCheckItem;
  acl: PrerequisiteCheckItem;
  quota: PrerequisiteCheckItem;
  hidepid: PrerequisiteCheckItem;
  tenant_group: PrerequisiteCheckItem;
  overall_ok: boolean;
}

export interface HardeningApplyRequest {
  enable_hidepid: boolean;
  ensure_tenant_group: boolean;
}

export interface HardeningApplyResponse {
  hidepid_applied: boolean;
  tenant_group_created: boolean;
  message: string;
}

// Site & Domain Types
export type RedirectMode = 'none' | 'www_to_non_www' | 'non_www_to_www' | 'custom_url';

export interface SiteDomain {
  id: number;
  site_id: number;
  name: string;
  unicode_name: string;
  is_primary: boolean;
  redirect_mode: RedirectMode | string;
  redirect_target: string | null;
  status: string;
  created_at: number;
  updated_at: number;
}

export interface Site {
  id: number;
  user_id: number;
  kind: 'static' | 'php' | 'proxy' | string;
  primary_domain: string;
  docroot: string;
  status: 'active' | 'disabled' | 'suspended' | string;
  spa_fallback: boolean;
  force_https: boolean;
  domains: SiteDomain[];
  runtime?: string | null;
  version?: string | null;
  preset?: string | null;
  created_at: number;
  updated_at: number;
}

export interface CreateSiteRequest {
  node_version?: string;
  python_version?: string;
  entrypoint_value?: string;
  php_version?: string;
  preset?: 'generic' | 'wordpress' | 'laravel' | 'custom';
  custom_entry_file?: string;
  domain: string;
  user_id?: number;
  spa_fallback?: boolean;
  force_https?: boolean;
  docroot?: string;
  kind?: 'static' | 'php' | 'nodejs' | 'python' | 'proxy' | string;
  runtime?: string;
  version?: string;
  port?: number;
  app_dir?: string;
  entrypoint?: string;
  entrypoint_type?: string;
  auto_restart?: boolean;
  env_vars?: Record<string, string>;
  framework?: string;
  server_type?: string;
  workers?: number;
  venv_dir?: string;
  static_url?: string;
  static_dir?: string;
  media_url?: string;
  media_dir?: string;
  upstream_url?: string;
  preserve_host?: boolean;
  websocket_support?: boolean;
  proxy_connect_timeout?: string;
  proxy_send_timeout?: string;
  proxy_read_timeout?: string;
  proxy_buffering?: boolean;
  custom_request_headers?: ProxyHeaderConfig[];
  custom_response_headers?: ProxyHeaderConfig[];
}

export interface AddDomainRequest {
  domain: string;
  redirect_mode?: RedirectMode | string;
  redirect_target?: string;
  is_primary?: boolean;
}

export interface UpdateDomainRequest {
  redirect_mode?: RedirectMode | string;
  redirect_target?: string;
  is_primary?: boolean;
}

export interface SiteMessageResponse {
  message: string;
  site_id: number;
  status: string;
}

// SSL & Certificate Types
export interface CertificateDetail {
  id: number;
  primary_domain: string;
  san_domains: string[];
  issuer: string;
  not_before: number;
  not_after: number;
  days_remaining: number;
  auto_renew: boolean;
  ocsp_stapling: boolean;
  last_renew_attempt: number | null;
  renew_error: string | null;
  status: string;
}

export interface SslStatus {
  has_ssl: boolean;
  certificate: CertificateDetail | null;
  force_https: boolean;
}

export interface IssueSslRequest {
  email?: string;
  staging?: boolean;
  fallback_self_signed?: boolean;
  ocsp_stapling?: boolean;
}

export interface CustomCertRequest {
  fullchain_pem: string;
  privkey_pem: string;
  auto_renew?: boolean;
}

export interface SslActionResponse {
  message: string;
  certificate?: CertificateDetail | null;
}

// Site Logs Types
export interface SiteLogsResponse {
  site_id: number;
  log_type: 'access' | 'error' | 'php_error' | 'php_slow' | string;
  lines: string[];
  total_returned: number;
  file_path: string;
  file_size_bytes: number;
}

export interface PhpVersionStatusItem {
  version: string;
  latest_release?: string | null;
  installed: boolean;
  fpm_binary_exists: boolean;
  cli_binary_exists: boolean;
  service_masked: boolean;
  status: string;
  fpm_binary_path: string;
  cli_binary_path: string;
  service_name: string;
}

export interface PhpRuntimeListApiResponse {
  ppa_configured: boolean;
  versions: PhpVersionStatusItem[];
}

export interface PhpExtensionItem {
  name: string;
  version: string;
  package_name: string;
  installed: boolean;
  enabled: boolean;
  is_base: boolean;
  category: string;
  description: string;
}

export interface PhpExtensionListApiResponse {
  version: string;
  extensions: PhpExtensionItem[];
}

export interface ToggleExtensionRequest {
  enabled: boolean;
}

export interface PhpExtensionActionApiResponse {
  version: string;
  extension: string;
  success: boolean;
  message: string;
}

export interface SitePhpExtensionsResponse {
  site_id: number;
  php_version: string;
  extensions: string[];
  available_extensions: string[];
}

export interface UpdateSitePhpExtensionsInput {
  extensions: string[];
}

export interface SwitchPhpVersionRequest {
  version: string;
}

export interface SwitchPhpVersionResponse {
  message: string;
  site_id: number;
  old_version: string;
  new_version: string;
  vhost_reloaded: boolean;
}

// PHP Settings Types
export interface PhpSettings {
  memory_limit_mb: number;
  upload_max_filesize_mb: number;
  post_max_size_mb: number;
  max_execution_time: number;
  max_input_vars: number;
  display_errors: boolean;
  date_timezone: string;
  opcache_enable: boolean;
  opcache_memory_consumption_mb: number;
  opcache_validate_timestamps: boolean;
  opcache_revalidate_freq: number;
}

export interface PhpSettingsLimits {
  memory_limit_mb: number;
  upload_limit_mb: number;
  max_execution_time: number;
  max_input_vars: number;
  opcache_memory_consumption_mb: number;
}

export interface SitePhpSettingsResponse {
  site_id: number;
  php_version: string;
  settings: PhpSettings;
  limits: PhpSettingsLimits;
  admin_ini?: string | null;
  is_admin: boolean;
}

export interface UpdateSitePhpSettingsInput {
  settings: PhpSettings;
  admin_ini?: string | null;
}

// PHP-FPM Status & Metrics Types
export interface PhpFpmStatusMetrics {
  pool: string;
  process_manager: string;
  start_time: number;
  start_since: number;
  accepted_conn: number;
  listen_queue: number;
  max_listen_queue: number;
  listen_queue_len: number;
  idle_processes: number;
  active_processes: number;
  total_processes: number;
  max_active_processes: number;
  max_children_reached: number;
  slow_requests: number;
}

export interface SitePhpStatusResponse {
  site_id: number;
  primary_domain: string;
  php_version: string;
  service_name: string;
  is_active: boolean;
  socket_exists: boolean;
  metrics: PhpFpmStatusMetrics | null;
}

export interface SitePhpLogsResponse {
  site_id: number;
  primary_domain: string;
  php_version: string;
  log_type: string;
  lines: string[];
  total_matched: number;
}

// SSH Key Types (Phase 6 - P6-T5)
export interface SshKeyItem {
  id: number;
  user_id: number;
  name: string;
  key_type: string;
  fingerprint_sha256: string;
  fingerprint_md5?: string | null;
  public_key: string;
  created_at: number;
}

export interface CreateSshKeyRequest {
  name: string;
  public_key: string;
}

export interface ValidateSshKeyRequest {
  public_key: string;
}

export interface ValidateSshKeyResponse {
  valid: boolean;
  key_type?: string | null;
  fingerprint_sha256?: string | null;
  fingerprint_md5?: string | null;
  comment?: string | null;
  bit_length?: number | null;
  error?: string | null;
}

export interface TenantShellPolicyResponse {
  user_id: number;
  username: string;
  shell_policy: string | null;
  effective_policy: string;
  inherited_from_plan: boolean;
  plan_policy: string | null;
}

export interface UpdateTenantShellPolicyRequest {
  shell_policy: string | null;
}

export interface SshDropinApplyRequest {
  custom_content?: string | null;
}

export interface SshDropinApplyResponse {
  success: boolean;
  path: string;
  validated: boolean;
  reloaded: boolean;
  message: string;
}

// SSH & SFTP Connection Details Types (Phase 6 - P6-T7)
export interface SshConnectionInfoQuery {
  site_id?: number;
  path?: string;
  tenant?: string;
  port?: number;
  host?: string;
}

export interface SshConnectionInfoResponse {
  host: string | null;
  port: number | null;
  username: string;
  user_id: number;
  home_dir: string;
  active_path: string;
  relative_path: string;
  shell_policy: 'full' | 'sftp' | 'none' | string;
  can_shell: boolean;
  can_sftp: boolean;
  has_ssh_keys: boolean;
  ssh_keys_count: number;
  ssh_command: string | null;
  sftp_command: string | null;
  sftp_cd_command: string | null;
  sftp_url: string | null;
  ssh_config_snippet: string | null;
  site_domain: string | null;
  endpoint_state: 'configured' | 'manual_preview' | 'missing' | 'unsupported_transport';
  endpoint_id: string | null;
  endpoint_revision: number | null;
  transport: string | null;
  host_keys: { key_type: string; fingerprint_sha256: string }[];
  host_keys_status: 'local_configuration' | 'unavailable' | 'untrusted_override' | 'not_applicable';
  sftp_path: string;
}

// Node.js Runtime Types (Phase 7 - P7-T1)
export interface NodeVersionItem {
  version: string;
  support_status?: 'lts' | 'eol' | 'not_lts' | 'unverified';
  raw_version: string;
  is_installed: boolean;
  installed_path: string | null;
  is_lts: boolean;
  lts_codename: string | null;
  release_date: string | null;
  npm_version: string | null;
  corepack_enabled: boolean;
  used_by_sites_count: number;
  used_by_domains: string[];
  can_delete: boolean;
}

export interface NodeRuntimeListApiResponse {
  mirror_url: string;
  versions: NodeVersionItem[];
  installed_count: number;
  is_offline_fallback: boolean;
}

export interface NodeRuntimeInstallRequest {
  version: string;
  mirror_url?: string;
  offline_tarball_path?: string;
  offline_checksum?: string;
  enable_corepack?: boolean;
}

export interface NodeRuntimeActionResponse {
  version: string;
  success: boolean;
  path: string;
  message: string;
}

// Node.js Site & Process Management Types (Phase 7 - P7-T2)
export interface NodeSiteStatusApiResponse {
  site_id: string;
  service_name: string;
  active: boolean;
  sub_state: string;
  pid: number | null;
  memory_bytes: number | null;
  cpu_usage_usec: number | null;
  uptime_secs: number | null;
  healthy: boolean;
  port: number;
  probe_status_code: number | null;
  probe_message: string | null;
}

export interface NodeSiteActionApiRequest {
  action: 'start' | 'stop' | 'restart' | 'redeploy' | string;
}

export interface NodeSiteJobApiRequest {
  job_type: 'npm_install' | 'npm_ci' | 'npm_build' | 'pnpm_install' | 'yarn_install' | 'custom' | string;
  script_name?: string;
  timeout_secs?: number;
}

export interface NodeSiteJobApiResponse {
  site_id: number;
  command: string;
  exit_code: number;
  success: boolean;
  stdout: string;
  stderr: string;
  duration_ms: number;
}

export interface NodeSiteSettingsDto {
  site_id: number;
  node_version: string;
  port: number;
  app_dir: string;
  entrypoint_type: 'script' | 'npm_start' | 'npm_custom' | string;
  entrypoint_value: string | null;
  auto_restart: boolean;
  env_vars: Record<string, string>;
}

export interface UpdateNodeSiteSettingsRequest {
  node_version?: string;
  port?: number;
  app_dir?: string;
  entrypoint_type?: string;
  entrypoint_value?: string;
  auto_restart?: boolean;
  env_vars?: Record<string, string>;
}

// Python Site & Process Management Types (Phase 7 - P7-T3)
export interface PythonRuntimeListApiResponse {
  versions: PythonVersionSummaryDto[];
  catalog_ready?: boolean;
}

export interface PythonVersionSummaryDto {
  version: string;
  major_minor: string;
  release_date: string | null;
  is_default: boolean;
  is_lts_or_stable: boolean;
  description: string;
  is_installed: boolean;
  install_path: string | null;
  bin_path: string | null;
}

export interface PythonSiteStatusApiResponse {
  site_id: string;
  service_name: string;
  active: boolean;
  sub_state: string;
  pid: number | null;
  memory_bytes: number | null;
  cpu_usage_usec: number | null;
  uptime_secs: number | null;
  socket_path: string;
  socket_exists: boolean;
  healthy: boolean;
  probe_message: string | null;
}

export interface PythonSiteActionApiRequest {
  action: 'start' | 'stop' | 'restart' | 'redeploy' | string;
}

export interface PythonSiteJobApiRequest {
  job_type: 'uv_venv' | 'uv_pip_install' | 'uv_pip_pyproject' | 'custom' | string;
  req_file?: string;
  script_name?: string;
  timeout_secs?: number;
}

export interface PythonSiteJobApiResponse {
  site_id: number;
  command: string;
  exit_code: number;
  success: boolean;
  stdout: string;
  stderr: string;
  duration_ms: number;
}

export interface PythonSiteSettingsDto {
  site_id: number;
  python_version: string;
  framework_preset: 'django' | 'flask' | 'fastapi' | 'generic' | string;
  server_type: 'gunicorn_wsgi' | 'uvicorn_asgi' | 'gunicorn_uvicorn_worker' | string;
  entrypoint_type: 'wsgi' | 'asgi' | 'custom' | string;
  entrypoint_value: string;
  workers: number;
  app_dir: string;
  venv_dir: string;
  socket_path: string;
  static_url: string | null;
  static_dir: string | null;
  media_url: string | null;
  media_dir: string | null;
  auto_restart: boolean;
  env_vars: Record<string, string>;
}

export interface UpdatePythonSiteSettingsRequest {
  python_version?: string;
  framework_preset?: string;
  server_type?: string;
  entrypoint_type?: string;
  entrypoint_value?: string;
  workers?: number;
  app_dir?: string;
  venv_dir?: string;
  static_url?: string;
  static_dir?: string;
  media_url?: string;
  media_dir?: string;
  auto_restart?: boolean;
  env_vars?: Record<string, string>;
}

// Reverse Proxy Types (Phase 7 - P7-T4)
export interface ProxyHeaderConfig {
  name: string;
  value: string;
}

export interface ProxySiteSettingsDto {
  site_id: number;
  upstream_url: string;
  preserve_host: boolean;
  websocket_support: boolean;
  proxy_connect_timeout: string | null;
  proxy_send_timeout: string | null;
  proxy_read_timeout: string | null;
  proxy_buffering: boolean | null;
  custom_request_headers: ProxyHeaderConfig[];
  custom_response_headers: ProxyHeaderConfig[];
  allow_external_upstream: boolean;
}

export interface UpdateProxySiteSettingsRequest {
  upstream_url?: string;
  preserve_host?: boolean;
  websocket_support?: boolean;
  proxy_connect_timeout?: string;
  proxy_send_timeout?: string;
  proxy_read_timeout?: string;
  proxy_buffering?: boolean;
  custom_request_headers?: ProxyHeaderConfig[];
  custom_response_headers?: ProxyHeaderConfig[];
}

export interface ProxyUpstreamProbeResponse {
  healthy: boolean;
  status_code: number | null;
  latency_ms: number;
  message: string;
}

// Unified Process Management Types (Phase 7 - P7-T5)
export interface ProcessItemDto {
  site_id: number;
  primary_domain: string;
  runtime: string;
  version?: string | null;
  service_name: string;
  active: boolean;
  sub_state: string;
  pid?: number | null;
  memory_bytes?: number | null;
  cpu_usage_usec?: number | null;
  uptime_secs?: number | null;
  healthy: boolean;
  port_or_socket?: string | null;
  probe_message?: string | null;
}

export interface ProcessListResponse {
  processes: ProcessItemDto[];
  total_active: number;
  total_memory_bytes: number;
}

// Environment Variables Types (Phase 8 - P8-T1)
export interface EnvVariableItem {
  key: string;
  value: string;
  is_masked: boolean;
}

export interface SiteEnvResponse {
  site_id: number;
  variables: EnvVariableItem[];
  restart_required: boolean;
}

export interface UpdateSiteEnvRequest {
  variables: Record<string, string>;
}

export interface ImportDotEnvRequest {
  content: string;
  merge?: boolean;
}

export interface SiteEnvActionResponse {
  success: boolean;
  message: string;
  restart_required: boolean;
}

// Cron Jobs Types (Phase 8 - P8-T2)
export interface CronJob {
  id: number;
  site_id: number;
  name: string;
  command: string;
  schedule: string;
  systemd_calendar: string;
  human_description: string;
  human_description_en: string;
  description?: string | null;
  work_dir?: string | null;
  timeout_secs: number;
  enabled: boolean;
  status: 'idle' | 'running' | 'success' | 'failed' | string;
  last_run_at?: number | null;
  last_duration_ms?: number | null;
  last_exit_code?: number | null;
  created_at: number;
  updated_at: number;
}

export interface CronQuota {
  used: number;
  limit: number;
}

export interface CronJobListResponse {
  jobs: CronJob[];
  quota: CronQuota;
}

export interface CreateCronJobRequest {
  name: string;
  command: string;
  schedule: string;
  description?: string;
  work_dir?: string;
  timeout_secs?: number;
  enabled?: boolean;
}

export interface UpdateCronJobRequest {
  name?: string;
  command?: string;
  schedule?: string;
  description?: string;
  work_dir?: string;
  timeout_secs?: number;
  enabled?: boolean;
}

export interface CronRun {
  id: number;
  cron_job_id: number;
  status: 'running' | 'success' | 'failed' | string;
  exit_code?: number | null;
  stdout?: string | null;
  stderr?: string | null;
  duration_ms?: number | null;
  started_at: number;
  finished_at?: number | null;
}

export interface ValidateCronResponse {
  valid: boolean;
  systemd_calendar?: string | null;
  human_id?: string | null;
  human_en?: string | null;
  error?: string | null;
}

// Site Rules (Phase 8 - P8-T3)
export type RedirectMatchType = 'exact' | 'prefix' | 'regex';

export interface StructuredRedirect {
  id: string;
  source: string;
  target: string;
  status_code: number;
  match_type: RedirectMatchType;
  enabled: boolean;
}

export interface CustomHeader {
  id: string;
  name: string;
  value: string;
  always: boolean;
  enabled: boolean;
}

export interface SecurityHeadersConfig {
  hsts_enabled: boolean;
  hsts_subdomains: boolean;
  hsts_preload: boolean;
  x_frame_options: string | null;
  x_content_type_options: boolean;
  referrer_policy: string | null;
  permissions_policy: string | null;
  csp: string | null;
}

export type CorsPreset = 'disabled' | 'allow_all' | 'same_origin' | 'custom';

export interface CorsConfig {
  enabled: boolean;
  preset: CorsPreset;
  allowed_origins: string[];
  allow_credentials: boolean;
  allowed_methods: string[];
  allowed_headers: string[];
  exposed_headers: string[];
  max_age_secs: number;
}

export type HotlinkAction = 'deny' | 'redirect';

export interface HotlinkConfig {
  enabled: boolean;
  allowed_domains: string[];
  file_extensions: string[];
  allow_empty_referer: boolean;
  action: HotlinkAction;
  redirect_url?: string | null;
}

export interface BasicAuthUser {
  username: string;
  password_hash: string;
}

export interface BasicAuthConfig {
  enabled: boolean;
  path: string;
  realm: string;
  users: BasicAuthUser[];
}

export type IpAction = 'allow' | 'deny';

export interface IpAccessRule {
  id: string;
  cidr_or_ip: string;
  action: IpAction;
  description?: string | null;
}

export interface IpAccessConfig {
  enabled: boolean;
  scope: 'global' | 'path';
  default_action: IpAction;
  path?: string | null;
  rules: IpAccessRule[];
}

export interface MaintenanceConfig {
  enabled: boolean;
  bypass_ips: string[];
  retry_after_secs: number;
  custom_message: string;
}

export interface CustomErrorPage {
  status_code: number;
  target: string;
}

export interface NginxSnippetConfig {
  enabled: boolean;
  content: string;
}

export interface SiteRulesConfig {
  redirects: StructuredRedirect[];
  custom_headers: CustomHeader[];
  security_headers: SecurityHeadersConfig;
  cors: CorsConfig;
  hotlink_protection: HotlinkConfig;
  basic_auth: BasicAuthConfig;
  ip_access: IpAccessConfig;
  maintenance: MaintenanceConfig;
  custom_error_pages: CustomErrorPage[];
  nginx_snippet: NginxSnippetConfig;
}

export interface HashPasswordResponse {
  hash: string;
}

export interface ValidateSnippetResponse {
  valid: boolean;
  message?: string | null;
  error?: string | null;
}

export interface SiteGitConfig {
  id?: number | null;
  site_id: number;
  repo_url: string;
  branch: string;
  build_cmd?: string | null;
  post_deploy_cmd?: string | null;
  docroot_subpath?: string | null;
  auto_deploy: boolean;
  keep_releases: number;
  deploy_key_public?: string | null;
  webhook_url: string;
  webhook_secret?: string | null;
  has_webhook_secret: boolean;
  last_deployed_at?: number | null;
  created_at?: number | null;
  updated_at?: number | null;
}

export interface UpsertGitConfigRequest {
  repo_url: string;
  branch: string;
  build_cmd?: string | null;
  post_deploy_cmd?: string | null;
  docroot_subpath?: string | null;
  auto_deploy?: boolean;
  keep_releases?: number;
  webhook_secret?: string | null;
  rotate_webhook_secret?: boolean;
}

export interface TriggerDeployRequest {
  branch?: string;
  commit_hash?: string;
}

export interface RollbackDeployRequest {
  target_release_id: string;
}

export interface SiteDeployment {
  id: number;
  site_id: number;
  release_id: string;
  commit_hash?: string | null;
  commit_author?: string | null;
  commit_message?: string | null;
  branch: string;
  status: 'queued' | 'running' | 'success' | 'failed' | string;
  duration_ms?: number | null;
  trigger_type: 'manual' | 'webhook' | 'rollback' | string;
  log?: string | null;
  error_message?: string | null;
  is_current: boolean;
  created_at: number;
  finished_at?: number | null;
}

export interface GenerateDeployKeyResponse {
  site_id: number;
  public_key: string;
}

// ==========================================
// Database Provisioner Types (Phase 8 - P8-T5)
// ==========================================

export type DatabaseEngine = 'mariadb' | 'postgresql';

export interface AvailableDatabaseEngine {
  engine: string;
  label: string;
  port: number;
}

export interface DatabaseServer {
  engine: DatabaseEngine;
  is_installed: boolean;
  is_running: boolean;
  version?: string | null;
  port: number;
  socket_path?: string | null;
  updated_at: number;
}

export interface DatabaseQuotaInfo {
  used: number;
  limit: number;
  remaining: number | null;
}

export interface DatabaseItem {
  id: number;
  user_id: number;
  site_id?: number | null;
  engine: DatabaseEngine;
  name: string;
  db_user: string;
  remote_access: boolean;
  allowed_hosts: string;
  collation: string;
  character_set: string;
  size_bytes: number;
  created_at: number;
  updated_at: number;
}

export interface DatabaseListResponse {
  databases: DatabaseItem[];
  total: number;
  quota: DatabaseQuotaInfo;
}

export interface CreateDatabaseRequest {
  engine: string;
  name: string;
  db_user?: string | null;
  password?: string | null;
  site_id?: number | null;
  remote_access?: boolean;
  allowed_hosts?: string | null;
  collation?: string | null;
  character_set?: string | null;
}

export interface CreateDatabaseResponse {
  database: DatabaseItem;
  generated_password?: string | null;
}

export interface RotatePasswordRequest {
  new_password?: string | null;
}

export interface RotatePasswordResponse {
  success: boolean;
  new_password: string;
  message: string;
}

export interface UpdateRemoteAccessRequest {
  remote_access: boolean;
  allowed_hosts: string;
}

export interface DatabaseActionResponse {
  success: boolean;
  message: string;
  job_id?: string | null;
}

export interface ExportDatabaseRequest {
  export_file_path?: string;
}

export interface ImportDatabaseRequest {
  import_file_path: string;
}

// Site Backup & Snapshot Restore Types (Phase 8 - P8-T6)
export type BackupFormat = 'tar.zst' | 'tar.gz';
export type BackupStatus = 'pending' | 'running' | 'completed' | 'failed' | 'restoring';
export type BackupScheduleFrequency = 'daily' | 'weekly' | 'monthly';
export type BackupStorageType = 'local' | 's3';

export interface BackupItem {
  id: number;
  site_id: number;
  user_id: number;
  filename: string;
  filepath: string;
  size_bytes: number;
  label?: string | null;
  includes_files: boolean;
  includes_databases: boolean;
  status: BackupStatus;
  error_message?: string | null;
  storage_type: BackupStorageType;
  s3_key?: string | null;
  checksum_sha256?: string | null;
  created_at: number;
  updated_at: number;
}

export interface BackupListResponse {
  backups: BackupItem[];
  total_bytes_used: number;
  count: number;
}

export interface CreateBackupRequest {
  label?: string | null;
  include_files?: boolean;
  include_databases?: boolean;
  format?: BackupFormat;
}

export interface RestoreBackupRequest {
  target_subpath?: string | null;
  restore_files?: boolean;
  restore_databases?: boolean;
}

export interface BackupSchedule {
  site_id: number;
  is_enabled: boolean;
  frequency: BackupScheduleFrequency;
  time_hour: number;
  time_minute: number;
  retention_count: number;
  include_files: boolean;
  include_databases: boolean;
  last_run_at?: number | null;
  next_run_at?: number | null;
  storage_destination: BackupStorageType;
  s3_endpoint?: string | null;
  s3_bucket?: string | null;
  s3_region?: string | null;
  s3_access_key?: string | null;
  s3_secret_key_enc?: string | null;
  s3_prefix?: string | null;
  updated_at: number;
}

export interface UpdateScheduleRequest {
  is_enabled: boolean;
  frequency: BackupScheduleFrequency;
  time_hour: number;
  time_minute: number;
  retention_count: number;
  include_files: boolean;
  include_databases: boolean;
  storage_destination?: string | null;
  s3_endpoint?: string | null;
  s3_bucket?: string | null;
  s3_region?: string | null;
  s3_access_key?: string | null;
  s3_secret_key?: string | null;
  s3_prefix?: string | null;
}

export type NotificationCategory =
  | 'ssl_expiring'
  | 'quota_warning'
  | 'job_failed'
  | 'deploy_failed'
  | 'backup_failed'
  | 'security_alert'
  | 'system';

export type NotificationSeverity = 'info' | 'warning' | 'critical';

export interface NotificationItem {
  id: string;
  user_id: number | null;
  category: NotificationCategory;
  severity: NotificationSeverity;
  title: string;
  message: string;
  action_url: string | null;
  is_read: boolean;
  read_at: string | null;
  created_at: string;
}

export interface ListNotificationsResponse {
  items: NotificationItem[];
  total: number;
  unread_count: number;
  page: number;
  per_page: number;
}

export interface NotificationSettings {
  id: number;
  smtp_enabled: boolean;
  smtp_host: string;
  smtp_port: number;
  smtp_username: string | null;
  smtp_password_masked: string | null;
  smtp_has_password: boolean;
  smtp_encryption: string;
  from_address: string;
  from_name: string;
  admin_email: string;
  ssl_warning_days: number;
  quota_warning_percent: number;
  notify_ssl_expiring: boolean;
  notify_quota_warning: boolean;
  notify_job_failed: boolean;
  notify_deploy_failed: boolean;
  updated_at: string;
}

export interface UpdateNotificationSettingsRequest {
  smtp_enabled?: boolean;
  smtp_host?: string;
  smtp_port?: number;
  smtp_username?: string | null;
  smtp_password?: string;
  smtp_encryption?: string;
  from_address?: string;
  from_name?: string;
  admin_email?: string;
  ssl_warning_days?: number;
  quota_warning_percent?: number;
  notify_ssl_expiring?: boolean;
  notify_quota_warning?: boolean;
  notify_job_failed?: boolean;
  notify_deploy_failed?: boolean;
}

export interface TestSmtpRequest {
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  encryption?: string;
  to_address?: string;
}

export interface TestSmtpResponse {
  success: boolean;
  message?: string;
  error?: string;
}

export interface EvaluateAlertsResponse {
  success: boolean;
  triggered_count: number;
  notifications: NotificationItem[];
}

export interface SiteLiveStatus {
  webserver_status: 'active' | 'suspended' | string;
  runtime_status: 'running' | 'inactive' | 'failed' | 'not_applicable' | string;
  healthy: boolean;
  uptime_secs?: number | null;
  pid?: number | null;
}

export interface SiteResourceMetrics {
  cpu_usage_usec: number;
  cpu_percent: number;
  memory_bytes: number;
  memory_limit_bytes: number;
  pids_current: number;
  pids_limit: number;
}

export interface SiteSslSummary {
  has_certificate: boolean;
  issuer?: string | null;
  valid_until?: number | null;
  days_remaining?: number | null;
  auto_renew: boolean;
  status: 'valid' | 'expiring_soon' | 'expired' | 'none' | string;
  force_https: boolean;
}

export interface SiteRuntimeSummary {
  kind: 'php' | 'nodejs' | 'python' | 'static' | 'proxy' | string;
  version: string;
  preset?: string | null;
  entrypoint_or_target: string;
  framework?: string | null;
}

export interface SiteStorageBreakdown {
  docroot_bytes: number;
  databases_bytes: number;
  backups_bytes: number;
  total_site_bytes: number;
  plan_disk_limit_bytes: number;
  usage_percentage: number;
}

export interface SiteContextualWarning {
  id: string;
  level: 'warning' | 'danger' | 'info';
  title: string;
  message: string;
  action_tab?: string | null;
}

export interface LatestDeploymentSummary {
  deployment_id: number;
  release_id: string;
  commit_hash: string;
  commit_message: string;
  commit_author: string;
  branch: string;
  trigger_type: string;
  status: string;
  duration_ms?: number | null;
  created_at: number;
  finished_at?: number | null;
  can_rollback: boolean;
}

export interface LatestBackupSummary {
  backup_id: number;
  filename: string;
  size_bytes: number;
  status: string;
  includes_files: boolean;
  includes_databases: boolean;
  created_at: number;
}

export interface SiteOverviewResponse {
  site_id: number;
  domain: string;
  status: SiteLiveStatus;
  quick_actions_available: string[];
  resources: SiteResourceMetrics;
  ssl: SiteSslSummary;
  runtime: SiteRuntimeSummary;
  storage: SiteStorageBreakdown;
  warnings: SiteContextualWarning[];
  latest_deployment?: LatestDeploymentSummary | null;
  latest_backup?: LatestBackupSummary | null;
}

export interface SiteQuickActionRequest {
  action: 'start' | 'stop' | 'restart' | 'reload' | 'clear_cache' | string;
}

export interface SiteQuickActionResponse {
  success: boolean;
  action: string;
  message: string;
}

// ==========================================
// Analytics & Timeseries Types (Phase 9 - P9-T5 & P9-T6)
// ==========================================

export interface AnalyticsTimeRangeQuery {
  from?: number;
  to?: number;
  interval?: 'auto' | '1m' | '1h' | '1d' | string;
  compare?: boolean;
  prev_from?: number;
  prev_to?: number;
}

export interface AnalyticsBreakdownsQuery {
  from?: number;
  to?: number;
  dimension?: string;
  limit?: number;
}

export interface AnalyticsSlowlogQuery {
  from?: number;
  to?: number;
  limit?: number;
}

export interface AnalyticsExportQuery {
  from?: number;
  to?: number;
  interval?: string;
}

export interface SiteAnalyticsSummaryDto {
  total_requests: number;
  total_bytes: number;
  unique_visitors: number;
  status_2xx: number;
  status_3xx: number;
  status_4xx: number;
  status_5xx: number;
  cache_hit_ratio: number;
  latency_avg_ms: number;
  latency_p50_ms: number;
  latency_p95_ms: number;
  latency_p99_ms: number;
  error_rate_percent: number;
}

export interface PeriodChangeDto {
  requests_change_percent: number;
  bytes_change_percent: number;
  visitors_change_percent: number;
  avg_latency_change_percent: number;
  error_rate_change_percent: number;
}

export interface SiteAnalyticsOverviewResponse {
  site_id: number;
  from_ts: number;
  to_ts: number;
  interval_used: string;
  current: SiteAnalyticsSummaryDto;
  previous?: SiteAnalyticsSummaryDto | null;
  comparison?: PeriodChangeDto | null;
}

export interface SiteTimeseriesPointDto {
  timestamp: number;
  requests: number;
  bytes: number;
  status_2xx: number;
  status_3xx: number;
  status_4xx: number;
  status_5xx: number;
  visitors: number;
  cache_hit: number;
  cache_miss: number;
  latency_p50_ms: number;
  latency_p95_ms: number;
  latency_p99_ms: number;
  latency_avg_ms: number;
}

export interface SiteTimeseriesResponse {
  site_id: number;
  from_ts: number;
  to_ts: number;
  interval: string;
  points: SiteTimeseriesPointDto[];
}

export interface BreakdownItemDto {
  name: string;
  count: number;
  percentage: number;
}

export interface SiteBreakdownsResponse {
  site_id: number;
  from_ts: number;
  to_ts: number;
  paths: BreakdownItemDto[];
  referrers: BreakdownItemDto[];
  countries: BreakdownItemDto[];
  browsers: BreakdownItemDto[];
  os: BreakdownItemDto[];
  devices: BreakdownItemDto[];
  slow_urls: BreakdownItemDto[];
  error_urls: BreakdownItemDto[];
}

export interface PhpFpmPoolOverviewDto {
  pool_name: string;
  active_avg: number;
  active_max: number;
  idle_avg: number;
  total_max: number;
  listen_queue_max: number;
  max_children_reached_max: number;
  slow_requests_total: number;
  accepted_conn_total: number;
}

export interface AppProcessOverviewDto {
  site_id: number;
  app_type: string;
  cpu_avg: number;
  cpu_max: number;
  memory_current_avg_bytes: number;
  memory_current_max_bytes: number;
  memory_peak_max_bytes?: number | null;
  pids_max: number;
  io_read_avg_bytes_sec: number;
  io_write_avg_bytes_sec: number;
}

export interface SiteAppMetricsResponse {
  site_id: number;
  runtime: string;
  php_fpm?: PhpFpmPoolOverviewDto | null;
  app_process?: AppProcessOverviewDto | null;
}

export interface PhpSlowlogFrameDto {
  frame_index: number;
  function: string;
  file: string;
  line: number;
  address?: string | null;
}

export interface PhpSlowlogEntryDto {
  id?: number | null;
  timestamp: number;
  pool: string;
  pid: number;
  script_filename: string;
  top_function?: string | null;
  frames: PhpSlowlogFrameDto[];
  duration_secs?: number | null;
  site_id?: number | null;
  tenant_user?: string | null;
}

export interface SiteSlowlogResponse {
  site_id: number;
  events: PhpSlowlogEntryDto[];
}

export interface ServerTimeseriesPointDto {
  timestamp: number;
  cpu_avg: number;
  cpu_max: number;
  memory_used_avg_bytes: number;
  memory_used_max_bytes: number;
  swap_used_avg_bytes: number;
  load1_avg: number;
  load1_max: number;
  disk_used_avg_bytes: number;
  net_rx_avg_bytes_sec: number;
  net_tx_avg_bytes_sec: number;
  io_read_avg_bytes_sec: number;
  io_write_avg_bytes_sec: number;
}

export interface SystemTimeseriesResponse {
  from_ts: number;
  to_ts: number;
  interval: string;
  points: ServerTimeseriesPointDto[];
}

export interface InsightActionDto {
  label: string;
  action_type: string;
  target_url?: string | null;
}

export interface AnalyticsInsightDto {
  id: string;
  site_id?: number | null;
  insight_type: string;
  severity: 'critical' | 'warning' | 'info';
  title: string;
  description: string;
  detected_at: number;
  action?: InsightActionDto | null;
  metadata?: Record<string, unknown> | null;
}

export interface SiteInsightsResponse {
  site_id: number;
  site_name: string;
  insights: AnalyticsInsightDto[];
  evaluated_at: number;
}

export interface SystemInsightsResponse {
  total_insights: number;
  insights: AnalyticsInsightDto[];
  evaluated_at: number;
}

// ==========================================
// Security Analytics & IP Banning (P9-T8)
// ==========================================

export type SecurityRiskLevel = 'low' | 'suspicious' | 'high_risk' | 'critical';

export interface TopSecurityIpDto {
  ip: string;
  total_requests: number;
  error_4xx: number;
  error_5xx: number;
  probing_count: number;
  auth_failures: number;
  country?: string | null;
  top_path?: string | null;
  risk_level: SecurityRiskLevel | string;
  last_seen: number;
}

export interface ProbingPathStatDto {
  path: string;
  category: string;
  count: number;
  unique_ips: number;
}

export interface BruteForceIncidentDto {
  ip: string;
  target_path: string;
  failed_attempts: number;
  first_attempt_ts: number;
  last_attempt_ts: number;
  country?: string | null;
  risk_score: number;
}

export interface SecurityOverviewResponse {
  site_id: number;
  from: number;
  to: number;
  total_probing_requests: number;
  total_brute_force_attempts: number;
  unique_suspicious_ips: number;
  top_ips: TopSecurityIpDto[];
  probing_paths: ProbingPathStatDto[];
  brute_force_incidents: BruteForceIncidentDto[];
  active_banned_ips_count: number;
}

export interface BannedIpDto {
  id: string;
  site_id: number;
  ip_address: string;
  reason: string;
  banned_at: number;
  expires_at?: number | null;
  remaining_ttl_secs?: number | null;
  is_active: boolean;
  banned_by: string;
}

export interface BanIpRequest {
  ip: string;
  ttl_secs?: number | null;
  reason?: string | null;
}

export interface UnbanIpResponse {
  success: boolean;
  message: string;
  unbanned_count: number;
}

// Analytics Retention & Data Lifecycle (Phase 9 - P9-T9)
export interface RetentionConfigDto {
  raw_retention_secs: number;
  rollup_1m_retention_secs: number;
  rollup_1h_retention_secs: number;
  rollup_1d_retention_secs?: number | null;
  security_events_retention_secs: number;
  max_db_size_bytes?: number | null;
  auto_vacuum: boolean;
}

export interface DatabaseTableStatDto {
  table_name: string;
  row_count: number;
}

export interface DatabaseStorageStatsDto {
  main_db_size_bytes: number;
  wal_size_bytes: number;
  total_size_bytes: number;
  page_size: number;
  page_count: number;
  freelist_count: number;
  free_bytes: number;
  fragmentation_ratio: number;
  tables: DatabaseTableStatDto[];
}

export interface RetentionMaintenanceReportDto {
  started_at: number;
  completed_at: number;
  pruned_raw_records: number;
  pruned_rollup_1m_records: number;
  pruned_rollup_1h_records: number;
  pruned_rollup_1d_records: number;
  pruned_security_events: number;
  pruned_stale_tailer_records: number;
  initial_db_size_bytes: number;
  final_db_size_bytes: number;
  bytes_reclaimed: number;
  vacuum_performed: boolean;
  wal_checkpoint_performed: boolean;
  is_emergency_pruning: boolean;
}

export interface IntegrityCheckReportDto {
  is_healthy: boolean;
  checked_at: number;
  errors: string[];
}

export interface AnalyticsRetentionResponse {
  // UI model. analyticsContracts maps the Rust retention/stats wire response.
  config: RetentionConfigDto;
  storage: DatabaseStorageStatsDto;
  last_report?: RetentionMaintenanceReportDto | null;
  integrity?: IntegrityCheckReportDto | null;
}

export interface UpdateRetentionRequest {
  raw_retention_secs?: number | null;
  rollup_1m_retention_secs?: number | null;
  rollup_1h_retention_secs?: number | null;
  rollup_1d_retention_secs?: number | null;
  security_events_retention_secs?: number | null;
  max_db_size_bytes?: number | null;
  auto_vacuum?: boolean | null;
}

export interface TriggerMaintenanceRequest {
  force_vacuum?: boolean | null;
}

// ==========================================
// Server Optimizer Types (Phase 10 - P10-T2)
// ==========================================

export interface ServerHardwareSpecsDto {
  vcpu_count: number;
  ram_total_mb: number;
  ram_free_mb: number;
  swap_total_mb: number;
  swap_free_mb: number;
  kernel_version: string;
  os_distro: string;
}

export interface NginxTuningConfigDto {
  worker_processes: string;
  worker_connections: number;
  worker_rlimit_nofile: number;
  multi_accept: boolean;
  keepalive_timeout_secs: number;
  client_body_buffer_size_kb: number;
  fastcgi_buffer_size_kb: number;
  fastcgi_buffers: string;
  open_file_cache_max: number;
  open_file_cache_inactive_secs: number;
}

export interface SysctlTuningConfigDto {
  somaxconn: number;
  tcp_max_syn_backlog: number;
  tcp_tw_reuse: number;
  ip_local_port_range: string;
  swappiness: number;
  file_max: number;
  netdev_max_backlog: number;
}

export interface PhpFpmTuningConfigDto {
  pm_type: string;
  max_children: number;
  start_servers?: number | null;
  min_spare_servers?: number | null;
  max_spare_servers?: number | null;
  process_idle_timeout_secs: number;
  max_requests: number;
}

export interface AppRuntimeHintsDto {
  recommended_node_workers: number;
  recommended_node_max_old_space_mb: number;
  recommended_python_workers: number;
}

export interface OptimizerProfileDto {
  profile_type: string;
  display_name: string;
  description: string;
  nginx: NginxTuningConfigDto;
  sysctl: SysctlTuningConfigDto;
  php_fpm: PhpFpmTuningConfigDto;
  hints: AppRuntimeHintsDto;
}

export interface CapacityMetric {
  value: number | null;
  sources: string[];
  unavailable_reason: string | null;
}

export interface CapacitySnapshot {
  schema_version: number;
  observed_at: number;
  scope: string;
  cpu_quota_percent: CapacityMetric;
  memory_limit_bytes: CapacityMetric;
  memory_available_bytes: CapacityMetric;
  disk_total_bytes: CapacityMetric;
  disk_available_bytes: CapacityMetric;
  inodes_total: CapacityMetric;
  inodes_available: CapacityMetric;
  notices: string[];
}

export interface OptimizerStatusResponseDto {
  capacity?: CapacitySnapshot | null;
  package_preset?: PackagePreset | null;
  hardware: ServerHardwareSpecsDto;
  recommended_profile: string;
  active_profile?: string | null;
  applied_at?: number | null;
  current_settings: OptimizerProfileDto;
  recommended_settings: OptimizerProfileDto;
  has_backup: boolean;
  last_backup_at?: number | null;
}

export interface PackageDemand {
  usage: 'personal' | 'multi_project' | 'multi_tenant';
  projects_per_tenant: number;
  tenant_slots: number;
}

export interface PackagePreset {
  rules_version: number;
  observed_at: number;
  expires_at: number;
  demand: PackageDemand;
  cpu_quota_percent: number;
  memory_limit_bytes: number;
  memory_high_bytes: number;
  disk_limit_bytes: number;
  inode_limit: number;
  tasks_max: number;
  io_weight: number;
  max_sites: number;
  max_upload_bytes: number;
  cpu_headroom_percent: number;
  memory_headroom_bytes: number;
  disk_headroom_bytes: number;
  notices: string[];
}

export interface ApplyOptimizerRequestDto {
  profile: string;
}

export interface OptimizerActionResponseDto {
  success: boolean;
  profile?: string | null;
  message: string;
  nginx_reloaded: boolean;
  sysctl_applied: boolean;
  timestamp: number;
}

// Memory Profile & PRD Section 9 Validation Types (Phase 10 - P10-T3)
export interface ProcessMemorySnapshot {
  pid: number;
  rss_bytes: number;
  rss_mb: number;
  vsize_bytes: number;
  vsize_mb: number;
  allocator: string;
  timestamp_epoch_secs: number;
}

export interface ReleaseProfileInfo {
  opt_level: string;
  lto: string;
  codegen_units: number;
  panic_strategy: string;
  strip: boolean;
  memory_allocator: string;
}

export interface MemoryTargetEvaluation {
  target_id: string;
  name: string;
  threshold: string;
  measured: string;
  passed: boolean;
  notes: string;
}

export interface PanelMemoryProfileReport {
  generated_at: string;
  allocator: string;
  release_profile: ReleaseProfileInfo;
  api_process?: ProcessMemorySnapshot | null;
  agent_process?: ProcessMemorySnapshot | null;
  total_panel_rss_mb: number;
  bundle_gzip_kb: number;
  cold_start_api_ms?: number | null;
  api_read_latency_p95_ms?: number | null;
  targets: MemoryTargetEvaluation[];
  all_passed: boolean;
  summary: string;
}

// Security Hardening & OWASP ASVS L2 / nftables Types (Phase 10 - P10-T4)
export interface AsvsRequirement {
  id: string;
  chapter: string;
  title: string;
  level: 'L1' | 'L2' | 'L3';
  status: 'Pass' | 'Fail' | 'Conditional' | 'NotApplicable';
  description: string;
  implementation_evidence: string;
  verification_notes?: string | null;
}

export interface OwaspAsvsAuditReport {
  audited_at: number;
  panel_version: string;
  asvs_version: string;
  target_level: string;
  total_controls: number;
  passed_controls: number;
  failed_controls: number;
  compliance_percentage: number;
  requirements: AsvsRequirement[];
  executive_summary: string;
}

export interface SystemdDirective {
  name: string;
  expected_setting: string;
  detected_setting?: string | null;
  compliant: boolean;
  risk_weight: number;
  description: string;
}

export interface SystemdServiceSecurityScore {
  service_name: string;
  exposure_score: number;
  rating: string;
  is_root: boolean;
  sandboxed: boolean;
  directives: SystemdDirective[];
}

export interface NftablesStatusResponseDto {
  enabled: boolean;
  table_name: string;
  active_rules_count: number;
  protected_ports_count: number;
  last_synced_at: number;
}

export interface TenantPortBindingDto {
  site_id: number;
  tenant_uid: number;
  port: number;
}

export interface SecurityHardeningOverviewResponse {
  asvs_audit: OwaspAsvsAuditReport;
  nftables_status: NftablesStatusResponseDto;
  systemd_security: SystemdServiceSecurityScore[];
  secret_vault_active: boolean;
  secret_key_version: number;
}

export interface NftablesSyncPayload {
  custom_bindings?: TenantPortBindingDto[];
}

export interface NftablesSyncResponseDto {
  success: boolean;
  rules_count: number;
  message: string;
}

// System Reliability, Doctor, Reconcile, and Backup (P10-T5)
export type DoctorCheckStatus = 'pass' | 'warn' | 'fail';
export type DoctorCheckCategory =
  | 'os_kernel'
  | 'prerequisites'
  | 'services'
  | 'socket'
  | 'database'
  | 'filesystem'
  | 'firewall'
  | 'jobs';

export interface DoctorCheckItem {
  id: string;
  name: string;
  category: DoctorCheckCategory;
  status: DoctorCheckStatus;
  message: string;
  remediation_hint: string | null;
  fixable: boolean;
}

export interface DoctorReport {
  timestamp: number;
  checks: DoctorCheckItem[];
  total_passed: number;
  total_warnings: number;
  total_failures: number;
  overall_status: DoctorCheckStatus;
  healthy: boolean;
}

export type DriftType =
  | 'missing_nginx_config'
  | 'orphaned_nginx_config'
  | 'missing_docroot_directory'
  | 'directory_permission_mismatch'
  | 'nftables_binding_mismatch'
  | 'interrupted_job_stuck';

export interface DriftItem {
  id: string;
  drift_type: DriftType;
  resource_type: string;
  resource_id: string;
  description: string;
  desired_state: string;
  actual_state: string;
  remediation_action: string;
}

export interface ReconcileReport {
  timestamp: number;
  total_resources_checked: number;
  drifts: DriftItem[];
  is_clean: boolean;
}

export interface SystemBackupSummary {
  filename: string;
  filepath: string;
  created_at: number;
  size_bytes: number;
  checksum_sha256: string;
  files_count: number;
  verified: boolean;
}

export interface CreateSystemBackupPayload {
  filename?: string;
}

export interface RestoreSystemBackupPayload {
  filepath: string;
}

export interface SystemBackupVerificationResult {
  is_valid: boolean;
  manifest: unknown | null;
  errors: string[];
}



