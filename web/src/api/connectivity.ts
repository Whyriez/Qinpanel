import { request } from './client';

export const services = ['panel', 'website', 'ssh', 'database', 'imap', 'submission', 'smtp_inbound'] as const;
export type Service = typeof services[number];
export const transports = ['direct', 'cloudflare_http', 'cloudflare_access', 'private_network', 'provider'] as const;
export type Transport = typeof transports[number];
export type Scope = { kind: 'host' } | { kind: 'project'; site_id: number } | { kind: 'engine'; engine: 'mariadb' | 'postgresql' };
export interface EndpointConfig {
  service: Service; scope: Scope; transport: Transport; host: string; port: number;
  tls_mode: 'none' | 'tls' | 'starttls' | 'ssh'; certificate_id: number | null; enabled: boolean; primary: boolean;
}
export interface Endpoint extends EndpointConfig {
  id: string; revision: number; client_requirement: 'none' | 'cloudflared' | 'cloudflare_one'; created_at: number; updated_at: number;
}
export interface Evidence {
  id: number; endpoint_id: string; service: Service; endpoint_revision: number;
  kind: 'installation' | 'service' | 'route' | 'listener' | 'firewall' | 'provider' | 'dns' | 'tls' | 'authentication' | 'external_reachability';
  observation_point: 'local_server' | 'user_client' | 'external_probe'; observer_id: string;
  address_family: 'ipv4' | 'ipv6' | null; observed_at: number; expires_at: number;
  outcome: 'pass' | 'fail' | 'unknown'; reason: string;
  current: boolean; expired: boolean; revision_matches: boolean; suggested_action: string;
}
export interface EndpointSummary { endpoint: Endpoint; evidence: Evidence[] }
export function endpointConfig(e: EndpointConfig): EndpointConfig {
  return { service: e.service, scope: e.scope, transport: e.transport, host: e.host, port: e.port, tls_mode: e.tls_mode, certificate_id: e.certificate_id, enabled: e.enabled, primary: e.primary };
}
export interface ConnectivityOverview {
  can_manage: boolean; endpoints: EndpointSummary[];
  page: number; per_page: number; total: number; total_pages: number; service_counts: Partial<Record<Service, number>>;
  projects: { id: number; name: string }[];
  certificates: { id: number; site_id: number; name: string }[];
}
export type Preset = 'direct' | 'tunnel' | 'mixed';
export function newCreateKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
export const serviceName = (s: Service, en = false) => ({ panel: 'Panel', website: en ? 'Website' : 'Situs web', ssh: 'SSH / SFTP', database: 'Database', imap: 'IMAP', submission: en ? 'Email submission' : 'Pengiriman dari aplikasi email', smtp_inbound: en ? 'Inbound SMTP' : 'SMTP masuk' })[s];
export const transportName = (s: Transport, en = false) => ({ direct: en ? 'Direct' : 'Langsung', cloudflare_http: 'Cloudflare Tunnel web', cloudflare_access: 'Cloudflare Access', private_network: en ? 'Private network' : 'Jaringan privat', provider: en ? 'External provider' : 'Penyedia eksternal' })[s];
export function availableTransports(service: Service): Transport[] {
  if (service === 'panel' || service === 'website') return ['direct', 'cloudflare_http', 'private_network'];
  if (service === 'ssh') return ['direct', 'cloudflare_access', 'private_network'];
  if (service === 'database') return ['direct', 'cloudflare_access', 'private_network', 'provider'];
  return ['direct', 'private_network', 'provider'];
}
export function presetConfig(service: Service, preset: Preset): Pick<EndpointConfig, 'transport' | 'tls_mode' | 'certificate_id'> {
  const web = service === 'panel' || service === 'website';
  const transport: Transport = preset === 'direct' ? 'direct' : web ? 'cloudflare_http' : service === 'ssh' ? (preset === 'mixed' ? 'direct' : 'cloudflare_access') : service === 'database' ? 'private_network' : 'provider';
  return { transport, tls_mode: service === 'ssh' ? 'ssh' : service === 'submission' || service === 'smtp_inbound' ? 'starttls' : 'tls', certificate_id: null };
}
export function endpointAuthority(e: Pick<EndpointConfig, 'host' | 'port'>) {
  return `${e.host.includes(':') ? `[${e.host}]` : e.host}:${e.port}`;
}
export function isCurrentEvidence(e: Endpoint, report: Evidence, now = Date.now()) {
  return e.enabled && report.current && !report.expired && report.revision_matches
    && report.endpoint_revision === e.revision && report.observed_at * 1000 <= now && report.expires_at * 1000 > now;
}
const record = (v: unknown): Record<string, unknown> => typeof v === 'object' && v !== null ? v as Record<string, unknown> : {};
const positive = (v: unknown) => typeof v === 'number' && Number.isSafeInteger(v) && v > 0;
export function parseConnectivity(value: unknown): ConnectivityOverview {
  const v = record(value);
  const invalid = () => { throw new Error('Data konektivitas tidak valid. Muat ulang halaman atau perbarui panel.'); };
  if (typeof v.can_manage !== 'boolean' || !Array.isArray(v.endpoints) || !Array.isArray(v.projects) || !Array.isArray(v.certificates)) return invalid();
  if (!positive(v.page) || !positive(v.per_page) || (v.per_page as number) > 100 || typeof v.total !== 'number' || !Number.isSafeInteger(v.total) || v.total < v.endpoints.length
    || typeof v.total_pages !== 'number' || !Number.isSafeInteger(v.total_pages) || v.total_pages < 0 || typeof v.service_counts !== 'object' || !v.service_counts) return invalid();
  if (Object.entries(v.service_counts).some(([key, count]) => !services.includes(key as Service) || typeof count !== 'number' || !Number.isSafeInteger(count) || count < 0)) return invalid();
  for (const item of v.endpoints) {
    const row = record(item), e = record(row.endpoint), scope = record(e.scope);
    if (typeof e.id !== 'string' || !positive(e.revision) || !services.includes(e.service as Service) || !transports.includes(e.transport as Transport)
      || typeof e.host !== 'string' || !e.host || !positive(e.port) || (e.port as number) > 65535 || typeof e.enabled !== 'boolean' || typeof e.primary !== 'boolean'
      || !['none', 'tls', 'starttls', 'ssh'].includes(e.tls_mode as string) || !['none', 'cloudflared', 'cloudflare_one'].includes(e.client_requirement as string)
      || !(e.certificate_id === null || positive(e.certificate_id)) || !Array.isArray(row.evidence)) return invalid();
    if (!(scope.kind === 'host' || (scope.kind === 'project' && positive(scope.site_id)) || (scope.kind === 'engine' && ['mariadb', 'postgresql'].includes(scope.engine as string)))) return invalid();
    for (const value of row.evidence) {
      const r = record(value);
      if (!positive(r.id) || r.endpoint_id !== e.id || r.service !== e.service || !positive(r.endpoint_revision)
        || !['installation', 'service', 'route', 'listener', 'firewall', 'provider', 'dns', 'tls', 'authentication', 'external_reachability'].includes(r.kind as string)
        || !['local_server', 'user_client', 'external_probe'].includes(r.observation_point as string) || typeof r.observer_id !== 'string'
        || !(r.address_family === null || ['ipv4', 'ipv6'].includes(r.address_family as string)) || typeof r.observed_at !== 'number' || typeof r.expires_at !== 'number'
        || !['pass', 'fail', 'unknown'].includes(r.outcome as string) || typeof r.reason !== 'string' || typeof r.suggested_action !== 'string'
        || typeof r.current !== 'boolean' || typeof r.expired !== 'boolean' || typeof r.revision_matches !== 'boolean') return invalid();
    }
  }
  if (v.projects.some(x => !positive(record(x).id) || typeof record(x).name !== 'string')
    || v.certificates.some(x => !positive(record(x).id) || !positive(record(x).site_id) || typeof record(x).name !== 'string')) return invalid();
  return value as ConnectivityOverview;
}
export const fetchConnectivity = async (page = 1, service: Service | '' = '', siteId?: number) => parseConnectivity(await request<unknown>(`/api/v1/connectivity?page=${page}&per_page=25${service ? `&service=${service}` : ''}${siteId === undefined ? '' : `&site_id=${siteId}`}`));
export const fetchEndpoint = async (id: string) => {
  const e = await request<Endpoint>(`/api/v1/connectivity/${encodeURIComponent(id)}`);
  return parseConnectivity({ can_manage: false, endpoints: [{ endpoint: e, evidence: [] }], projects: [], certificates: [], page: 1, per_page: 25, total: 1, total_pages: 1, service_counts: {} }).endpoints[0].endpoint;
};
export const createEndpoint = (config: EndpointConfig, key: string) => request<Endpoint>('/api/v1/connectivity', { method: 'POST', headers: { 'Idempotency-Key': key }, body: JSON.stringify(endpointConfig(config)) });
export const replaceEndpoint = (e: Endpoint, config: EndpointConfig) => request<Endpoint>(`/api/v1/connectivity/${encodeURIComponent(e.id)}`, { method: 'PUT', body: JSON.stringify({ expected_revision: e.revision, config: endpointConfig(config) }) });
export const deleteEndpoint = (e: Endpoint) => request(`/api/v1/connectivity/${encodeURIComponent(e.id)}`, { method: 'DELETE', body: JSON.stringify({ expected_revision: e.revision }) });

export const diagnoseEndpoint = async (endpoint: Endpoint): Promise<EndpointSummary> => {
  const result = await request<EndpointSummary>(`/api/v1/connectivity/${encodeURIComponent(endpoint.id)}/diagnose`, { method: 'POST', body: JSON.stringify({ expected_revision: endpoint.revision }) });
  return parseConnectivity({ can_manage: false, endpoints: [result], projects: [], certificates: [], page: 1, per_page: 25, total: 1, total_pages: 1, service_counts: {} }).endpoints[0];
};
