import type { DatabaseItem, DatabaseListResponse } from '../types/api';

const invalidResponse = 'Format respons basis data tidak valid. Perbarui panel lalu coba lagi.';
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(invalidResponse);
  return value as Record<string, unknown>;
}
function integer(value: unknown, minimum = 0): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < minimum) throw new Error(invalidResponse);
  return value;
}

/** Normalize the Rust list contract before committing any data to component state. */
export function parseDatabaseList(value: unknown): DatabaseListResponse {
  const data = record(value);
  const total = integer(data.total);
  const used = integer(data.used_databases);
  // Rust treats a nonpositive plan limit as unlimited.
  const limit = integer(data.plan_max_databases, Number.MIN_SAFE_INTEGER);
  if (!Array.isArray(data.databases)) throw new Error(invalidResponse);
  const databases = data.databases.map(value => {
    const item = record(value);
    integer(item.id, 1);
    integer(item.user_id, 1);
    if (item.site_id != null) integer(item.site_id, 1);
    if (!['mariadb', 'postgresql'].includes(String(item.engine))) throw new Error(invalidResponse);
    for (const key of ['name', 'db_user', 'allowed_hosts', 'collation', 'character_set']) {
      if (typeof item[key] !== 'string') throw new Error(invalidResponse);
    }
    if (typeof item.remote_access !== 'boolean') throw new Error(invalidResponse);
    integer(item.size_bytes);
    integer(item.created_at);
    integer(item.updated_at);
    return item as unknown as DatabaseItem;
  });
  return { databases, total, quota: { used, limit, remaining: limit > 0 ? Math.max(0, limit - used) : null } };
}
