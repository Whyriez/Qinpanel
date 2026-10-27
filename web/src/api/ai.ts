import { request } from './client';
import type { PackageDemand, PackagePreset } from '../types/api';

export type AiProvider = 'gemini' | 'openai';
export interface AiSettings {
  enabled: boolean;
  provider: AiProvider;
  model: string;
  revision: number;
  daily_request_limit: number;
  requests_today: number;
  max_output_tokens: number;
  reset_timezone: 'UTC';
  providers: { id: AiProvider; models: string[]; has_key: boolean }[];
}
export interface AiConfiguration {
  expected_revision: number;
  enabled: boolean;
  provider: AiProvider;
  model: string;
  daily_request_limit: number;
  api_key?: string;
  remove_api_key?: boolean;
}
export interface AiRecommendation {
  id: string;
  provider: AiProvider;
  model: string;
  settings_revision: number;
  snapshot_fingerprint: string;
  profile: 'conservative' | 'balanced' | 'performance';
  summary: string;
  considerations: string[];
  preset: PackagePreset;
  total_tokens: number | null;
  draft_only: true;
}
const providers = new Set(['gemini', 'openai']);
const positive = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
export async function fetchAiSettings(signal?: AbortSignal): Promise<AiSettings> {
  const data = await request<AiSettings>('/api/v1/system/ai', { signal });
  if (!data || !providers.has(data.provider) || !positive(data.revision) || typeof data.enabled !== 'boolean'
    || !Array.isArray(data.providers) || !data.providers.length
    || data.providers.some(p => !providers.has(p.id) || !Array.isArray(p.models) || !p.models.length || p.models.some(m => typeof m !== 'string') || typeof p.has_key !== 'boolean')) {
    throw new Error('Respons integrasi AI tidak valid. Perbarui paket Qinpanel lengkap.');
  }
  return data;
}
export function saveAiSettings(config: AiConfiguration): Promise<{ success: boolean; revision: number }> {
  return request('/api/v1/system/ai', { method: 'PUT', body: JSON.stringify(config) });
}
export function testAiConnection(revision: number): Promise<{ success: boolean; total_tokens: number | null }> {
  return request('/api/v1/system/ai/test', { method: 'POST', body: JSON.stringify({ expected_revision: revision, consent: true }) });
}
export function validAiDraft(value: AiRecommendation, now = Date.now() / 1000): boolean {
  const p = value?.preset;
  const d = p?.demand;
  return Boolean(value?.draft_only === true && providers.has(value.provider) && positive(value.settings_revision)
    && typeof value.id === 'string' && value.id.length > 0 && /^[a-f0-9]{64}$/.test(value.snapshot_fingerprint)
    && ['conservative', 'balanced', 'performance'].includes(value.profile)
    && typeof value.summary === 'string' && Array.isArray(value.considerations) && value.considerations.every(v => typeof v === 'string')
    && p && p.rules_version === 1 && positive(p.observed_at) && positive(p.expires_at) && p.observed_at <= now && now < p.expires_at && p.expires_at - p.observed_at <= 60
    && d && positive(d.projects_per_tenant) && d.projects_per_tenant <= 256 && positive(d.tenant_slots) && d.tenant_slots <= 10000
    && ((d.usage === 'personal' && d.projects_per_tenant === 1 && d.tenant_slots === 1)
      || (d.usage === 'multi_project' && d.projects_per_tenant >= 2 && d.tenant_slots === 1)
      || (d.usage === 'multi_tenant' && d.tenant_slots >= 2))
    && [p.cpu_quota_percent, p.memory_limit_bytes, p.memory_high_bytes, p.disk_limit_bytes, p.inode_limit, p.tasks_max, p.io_weight, p.max_sites, p.max_upload_bytes].every(positive)
    && p.memory_high_bytes < p.memory_limit_bytes && p.max_upload_bytes <= p.disk_limit_bytes);
}
export async function recommendAiPackage(description: string, demand: PackageDemand, revision: number, english: boolean, signal?: AbortSignal): Promise<AiRecommendation> {
  const result = await request<AiRecommendation>('/api/v1/plans/ai/recommend', { method: 'POST', signal,
    body: JSON.stringify({ description, demand, expected_revision: revision, consent: true, english }) });
  if (!validAiDraft(result)) throw new Error('Preview AI sudah usang atau tidak valid. Buat analisis baru.');
  return result;
}

/** Only allowlisted resource fields enter a NEW manual form. Never permissions. */
export function aiDraftFields(result: AiRecommendation) {
  if (!validAiDraft(result)) throw new Error('Preview AI sudah kedaluwarsa. Buat analisis baru.');
  const p = result.preset;
  const mib = 1024 * 1024;
  return {
    cpu_quota_percent: p.cpu_quota_percent,
    memory_limit_mb: Math.floor(p.memory_limit_bytes / mib),
    memory_high_mb: Math.floor(p.memory_high_bytes / mib),
    disk_limit_mb: Math.floor(p.disk_limit_bytes / mib),
    inode_limit: p.inode_limit,
    tasks_max: p.tasks_max,
    io_weight: p.io_weight,
    max_sites: p.max_sites,
    max_domains: Math.min(1000, p.max_sites * 2),
    max_databases: p.max_sites,
    max_upload_mb: Math.floor(p.max_upload_bytes / mib),
  };
}
