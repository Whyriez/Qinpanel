import type { AuditCategory } from '../types/api';

export const auditCategories = ['authentication', 'users', 'websites', 'files', 'databases', 'runtimes', 'access', 'network', 'email', 'backups', 'system', 'other'] as const satisfies readonly AuditCategory[];
export type AuditFilters = { category: AuditCategory | ''; q: string; status: '' | 'success' | 'failure'; page: number };

export function isAuditCategory(value: unknown): value is AuditCategory {
  return auditCategories.some(category => category === value);
}

export function parseAuditFilters(params: URLSearchParams): AuditFilters {
  const category = params.get('category');
  const status = params.get('status');
  const page = Number(params.get('page') || 1);
  return {
    category: isAuditCategory(category) ? category : '',
    q: (params.get('q') || '').slice(0, 200),
    status: status === 'success' || status === 'failure' ? status : '',
    page: Number.isSafeInteger(page) && page > 0 ? page : 1,
  };
}

export function auditSearchParams(filters: AuditFilters): string {
  const params = new URLSearchParams();
  if (filters.category) params.set('category', filters.category);
  if (filters.q.trim()) params.set('q', filters.q.trim());
  if (filters.status) params.set('status', filters.status);
  if (filters.page > 1) params.set('page', String(filters.page));
  const value = params.toString();
  return value ? `?${value}` : '';
}

export function auditDetails(value: string | null): string | null {
  if (value === null || value === '') return null;
  try { return JSON.stringify(JSON.parse(value), null, 2); }
  catch { return value; }
}
