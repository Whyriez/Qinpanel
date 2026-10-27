import { errorMessage, getCsrfToken, request } from './client';

export type StorageProvider = 'aws' | 'r2' | 'supabase' | 'compatible';
export interface StorageConnection {
  id: string; owner_id: number; site_id: number | null; label: string;
  provider: StorageProvider; endpoint: string; region: string;
  addressing: 'path' | 'virtual_host'; bucket: string; revision: number;
  conditional_put?: boolean;
  managed?: boolean;
  managed_state?: 'pending' | 'ready' | 'unknown' | 'deleting' | 'deleted' | null;
  application_key_generation?: number;
  application_key_state?: 'pending' | 'ready' | 'unknown' | null;
  last_operation?: 'running' | 'unknown' | 'succeeded' | 'failed' | null;
  verified_at: number | null; has_credentials: boolean; created_at: number; updated_at: number;
}
export interface StorageContext { owners: { id: number; username: string }[]; sites: { id: number; owner_id: number; domain: string }[]; upload_limit_bytes: number }
export interface ConnectionInput {
  expected_revision: number; owner_id: number; site_id: number | null; label: string;
  provider: StorageProvider; endpoint: string; region: string; addressing: 'path' | 'virtual_host'; bucket: string;
  credentials?: { access_key: string; secret_key: string; session_token: string | null };
}
export interface StorageObject { key: string; size: number; modified_at: string }
export interface StorageListing { objects: StorageObject[]; prefixes: string[]; next_token: string | null }
const root = '/api/v1/storage';
const path = (id: string) => `${root}/connections/${encodeURIComponent(id)}`;
export const fetchStorageContext = (signal?: AbortSignal) => request<StorageContext>(`${root}/context`, { signal });
export const fetchStorageConnections = (site?: number, signal?: AbortSignal) => request<{ connections: StorageConnection[]; upload_limit_bytes: number }>(`${root}/connections${site ? `?site_id=${site}` : ''}`, { signal });
export const saveStorageConnection = (value: ConnectionInput, id?: string) => request<StorageConnection>(id ? path(id) : `${root}/connections`, { method: 'POST', body: JSON.stringify(value) });
export const testStorageConnection = (connection: StorageConnection) => request<{ success: boolean; revision: number; verified_at: number }>(`${path(connection.id)}/test`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision }) });
export const fetchStorageObjects = (connection: StorageConnection, prefix: string, token: string | null, signal?: AbortSignal) => {
  const query = new URLSearchParams({ prefix }); if (token) query.set('token', token);
  return request<{ listing: StorageListing; revision: number; verified_at: number }>(`${path(connection.id)}/objects?${query}`, { signal });
};
export const bindStorageProject = (connection: StorageConnection, site_id: number | null) => request(`${path(connection.id)}/binding`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision, site_id }) });
export const disconnectStorage = (connection: StorageConnection) => request(path(connection.id), { method: 'DELETE', body: JSON.stringify({ expected_revision: connection.revision, confirmation: connection.bucket }) });
export const createStorageBucket = (connection: StorageConnection) => request(`${path(connection.id)}/bucket`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision, confirmation: connection.bucket }) });
export interface ManagedStorageStatus {
  configured: boolean; running: boolean; endpoint: string | null; engine_version: string;
  data_directory: string; capacity_bytes: number | null; available_bytes: number | null; bucket_count: number; quota_enforcement: 'none';
}
export interface StorageKeys { access_key: string; secret_key: string; session_token: string | null }
// The initial HTTP/IP installation also needs cryptographic allocation IDs.
export function storageAllocationId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, value => value.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
export const fetchManagedStorage = (signal?: AbortSignal) => request<ManagedStorageStatus>(`${root}/managed`, { signal });
export const configureManagedStorage = (value: { hostname: string; fullchain_pem: string; private_key_pem: string }) => request<ManagedStorageStatus>(`${root}/managed`, { method: 'POST', body: JSON.stringify(value) });
export interface StorageTlsCertificate {
  revision: number; request_id: string | null; hostname: string | null; email: string | null;
  state: 'idle' | 'queued' | 'issuing' | 'configuring' | 'active' | 'failed' | 'manual';
  auto_renew: boolean; not_after: number | null; renew_at: number | null; job_id: string | null; error: string | null;
}
export interface StorageTlsInventory { certificate: StorageTlsCertificate; dns: { configured: boolean; zone_name: string | null } }
export async function fetchStorageTls(signal?: AbortSignal): Promise<StorageTlsInventory> {
  const value=await request<StorageTlsInventory>(`${root}/managed/tls`, { signal });
  if (!value?.certificate || !Number.isSafeInteger(value.certificate.revision) || value.certificate.revision < 1
    || !['idle','queued','issuing','configuring','active','failed','manual'].includes(value.certificate.state)
    || typeof value.certificate.auto_renew !== 'boolean' || typeof value.dns?.configured !== 'boolean') {
    throw new Error('Status sertifikat storage tidak valid. Perbarui paket Qinpanel lengkap dan muat ulang.');
  }
  return value;
}
export const generateStorageTls = (value: { request_id: string; expected_revision: number; hostname: string; email: string; confirmed: boolean }) => request<StorageTlsCertificate>(`${root}/managed/tls`, { method: 'POST', body: JSON.stringify(value) });
export const allocateManagedBucket = (value: { request_id: string; owner_id: number; site_id: number | null; label: string; bucket: string }) => request<StorageConnection>(`${root}/managed/buckets`, { method: 'POST', body: JSON.stringify(value) });
export const reconcileManagedBucket = (connection: StorageConnection) => request<StorageConnection>(`${path(connection.id)}/provision`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision }) });
export const removeManagedBucket = (connection: StorageConnection) => request(`${path(connection.id)}/decommission`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision, confirmation: connection.bucket }) });
export const issueStorageKey = (connection: StorageConnection, confirmation: string, retry: boolean) => request<{ credentials: StorageKeys; generation: number }>(`${path(connection.id)}/application-key`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision, expected_generation: connection.application_key_generation ?? 0, confirmation, retry }) });
export const deleteStorageObject = (connection: StorageConnection, key: string) => request(`${path(connection.id)}/delete`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision, key, confirmation: key }) });
export const downloadStorageObject = (connection: StorageConnection, key: string) => request<{ url: string; expires_at: number }>(`${path(connection.id)}/download`, { method: 'POST', body: JSON.stringify({ expected_revision: connection.revision, key }) });

export function validObjectKey(key: string): boolean {
  const control = Array.from(key).some(char => { const code = char.charCodeAt(0); return code < 32 || code >= 127 && code <= 159 || char === '\\'; });
  return key.length > 0 && new TextEncoder().encode(key).length <= 1024 && !control && !key.split('/').some(part => part === '.' || part === '..');
}
export function projectStorageEnvironment(connection: StorageConnection): string {
  // Configuration only: provider keys must be entered explicitly in project secrets.
  return [`AWS_ACCESS_KEY_ID=<access-key-id>`, `AWS_SECRET_ACCESS_KEY=<secret-access-key>`, `AWS_DEFAULT_REGION=${connection.region}`, `AWS_BUCKET=${connection.bucket}`, `AWS_ENDPOINT=${connection.endpoint}`, `AWS_USE_PATH_STYLE_ENDPOINT=${connection.addressing === 'path'}`].join('\n');
}
export function safeDownload(value: { url: string; expires_at: number }, connection: StorageConnection): string {
  if (value.expires_at <= Date.now() / 1000 || value.expires_at > Date.now() / 1000 + 65) throw new Error('Invalid or expired storage download');
  const panelPrefix = `/api/v1/storage/connections/${encodeURIComponent(connection.id)}/download?`;
  if (value.url.startsWith(panelPrefix)) {
    const url = new URL(value.url, 'https://panel.invalid');
    if (url.searchParams.get('expected_revision') !== String(connection.revision) || Number(url.searchParams.get('expires_at')) !== value.expires_at || !validObjectKey(url.searchParams.get('key') ?? '')) throw new Error('Invalid storage download');
    return value.url;
  }
  const url = new URL(value.url); const endpoint = new URL(connection.endpoint);
  const host = connection.addressing === 'virtual_host' ? `${connection.bucket}.${endpoint.hostname}` : endpoint.hostname;
  const prefix = endpoint.pathname.replace(/\/$/, '') + (connection.addressing === 'path' ? `/${connection.bucket}/` : '/');
  if (url.protocol !== 'https:' || url.username || url.password || url.host !== host || !url.pathname.startsWith(prefix) || value.expires_at <= Date.now() / 1000 || value.expires_at > Date.now() / 1000 + 65) throw new Error('Invalid or expired storage download');
  return url.href;
}
export function uploadStorageObject(connection: StorageConnection, file: File, key: string, overwrite: boolean, onProgress: (percent: number) => void, signal: AbortSignal): Promise<void> {
  if (!validObjectKey(key) || file.size > 32 * 1024 * 1024) return Promise.reject(new Error('Invalid object key or file exceeds 32 MiB'));
  const token = getCsrfToken(); if (!token) return Promise.reject(new Error('Session expired. Reload the panel.'));
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const abort = () => xhr.abort();
    const done = () => signal.removeEventListener('abort', abort);
    const query = new URLSearchParams({ key, expected_revision: String(connection.revision), overwrite: String(overwrite) });
    xhr.open('POST', `${path(connection.id)}/upload?${query}`); xhr.withCredentials = true; xhr.timeout = 180000;
    xhr.setRequestHeader('X-CSRF-Token', token); xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.upload.onprogress = event => { if (event.lengthComputable) onProgress(Math.floor(event.loaded / event.total * 100)); };
    xhr.onload = () => { done(); if (xhr.status >= 200 && xhr.status < 300) resolve(); else { let body: unknown; try { body = JSON.parse(xhr.responseText); } catch { body = null; } reject(new Error(errorMessage(body, 'Upload failed. Refresh objects before retrying.'))); } };
    xhr.onerror = xhr.ontimeout = () => { done(); reject(new Error('Connection interrupted. Refresh objects before retrying.')); };
    xhr.onabort = () => { done(); reject(new DOMException('Upload cancelled; refresh objects before retrying.', 'AbortError')); };
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) { done(); reject(new DOMException('Aborted', 'AbortError')); return; }
    xhr.send(file);
  });
}
