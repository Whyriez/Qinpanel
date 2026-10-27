import { request } from './client';

export type UpdatePhase = 'queued' | 'running' | 'succeeded' | 'failed' | 'interrupted';
export interface PanelRelease { version: string; digest: string; notes: string }
export interface UpdateRequest { id: string; version: string; digest: string }
export interface UpdateRun extends UpdateRequest { phase: UpdatePhase; started_at: number; finished_at: number | null }
export interface UpdateStatus { current_version: string; installed_digest: string | null; supported: boolean; checked_at: number | null; available: PanelRelease | null; check_error: string | null; update: UpdateRun | null; diagnostics?: string | null }
const tag = /^v(0|[1-9]\d{0,7})\.(0|[1-9]\d{0,7})\.(0|[1-9]\d{0,7})$/;
const digest = /^[a-f0-9]{64}$/;
export function isUpdating(run?: UpdateRun | null) { return run?.phase === 'queued' || run?.phase === 'running'; }
export function canUpdate(data: UpdateStatus, now = Date.now()) {
  const release = data.available;
  if (!release || !data.supported || data.check_error || isUpdating(data.update) || data.checked_at === null || now / 1000 - data.checked_at > 600 || now / 1000 < data.checked_at || release.digest === data.installed_digest) return false;
  const current = data.current_version.slice(1).split('.').map(Number), target = release.version.slice(1).split('.').map(Number);
  for (let i = 0; i < 3; i++) { if (target[i] !== current[i]) return target[i] > current[i]; }
  return data.current_version === 'v0.1.0';
}
export function parseUpdateStatus(value: unknown): UpdateStatus {
  const fail = () => { throw new Error('Status pembaruan tidak valid. Muat ulang status.'); };
  if (!value || typeof value !== 'object') return fail();
  const data = value as UpdateStatus;
  if (data.diagnostics !== undefined && data.diagnostics !== null && typeof data.diagnostics !== 'string') return fail();
  if (!tag.test(data.current_version) || typeof data.supported !== 'boolean' || !(data.installed_digest === null || (typeof data.installed_digest === 'string' && digest.test(data.installed_digest))) || !(data.checked_at === null || (Number.isSafeInteger(data.checked_at) && data.checked_at >= 0)) || !(data.check_error === null || typeof data.check_error === 'string')) return fail();
  if (data.available !== null && (!data.available || !tag.test(data.available.version) || !digest.test(data.available.digest) || typeof data.available.notes !== 'string')) return fail();
  if (data.update !== null && (!data.update || typeof data.update.id !== 'string' || !tag.test(data.update.version) || !digest.test(data.update.digest) || !['queued', 'running', 'succeeded', 'failed', 'interrupted'].includes(data.update.phase) || !Number.isSafeInteger(data.update.started_at) || !(data.update.finished_at === null || Number.isSafeInteger(data.update.finished_at)))) return fail();
  return data;
}
export const updateStatus = async (signal?: AbortSignal) => parseUpdateStatus(await request<unknown>('/api/v1/system/update', { signal }));
export const checkUpdate = async () => parseUpdateStatus(await request<unknown>('/api/v1/system/update/check', { method: 'POST' }));
export const updateLogs = async () => parseUpdateStatus(await request<unknown>('/api/v1/system/update/logs'));
export const applyUpdate = async (body: UpdateRequest) => parseUpdateStatus(await request<unknown>('/api/v1/system/update/apply', { method: 'POST', body: JSON.stringify(body) }));
