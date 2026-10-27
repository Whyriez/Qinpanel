import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Search, RefreshCw, ChevronLeft, ChevronRight, Eye, X, AlertCircle } from 'lucide-react';
import type { AuditLogItem, AuditLogListResponse } from '../types/api';
import { fetchAuditLogs } from '../api/client';
import { ModalOverlay } from '../components/ui/ModalOverlay';
import { StatusPill } from '../components/ui/StatusPill';
import { EmptyState } from '../components/ui/EmptyState';
import { auditCategories, auditDetails, auditSearchParams, isAuditCategory, parseAuditFilters, type AuditFilters } from '../lib/auditFilters';
import { navigate, useBrowserRoute } from '../lib/router';

const perPage = 15;
const control = 'audit-control rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] px-3 text-xs text-[var(--ink)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';

export function AuditLogPage() {
  const { t, i18n } = useTranslation();
  const { pathname, search } = useBrowserRoute();
  const { category, q, status, page } = parseAuditFilters(search);
  const [draft, setDraft] = useState(q);
  const [refresh, setRefresh] = useState(0);
  const [inspectLog, setInspectLog] = useState<AuditLogItem | null>(null);
  const [request, setRequest] = useState<{ key: string; result?: AuditLogListResponse; error?: string }>({ key: '' });
  const key = JSON.stringify([category, q, status, page, refresh]);
  const current = request.key === key ? request : undefined;
  const result = current?.result;
  const error = current?.error;
  const loading = !result && !error;
  const filtersActive = Boolean(category || q || status);

  const changeFilters = (patch: Partial<AuditFilters>) => {
    setInspectLog(null);
    navigate(`${pathname}${auditSearchParams({ category, q, status, page: 1, ...patch })}`);
  };
  const resetFilters = () => { setDraft(''); changeFilters({ category: '', q: '', status: '' }); };

  useEffect(() => { setDraft(q); }, [q]);
  useEffect(() => {
    let active = true;
    const controller = new AbortController();
    void fetchAuditLogs({ category: category || undefined, q: q || undefined, status: status || undefined, page, per_page: perPage }, controller.signal)
      .then(response => {
        if (!active) return;
        if (!Array.isArray(response.items) || !Number.isSafeInteger(response.total) || response.total < 0 || !Number.isSafeInteger(response.total_pages) || response.total_pages < 0) {
          throw new Error(t('audit.invalidResponse'));
        }
        if (page > Math.max(1, response.total_pages)) {
          navigate(`${pathname}${auditSearchParams({ category, q, status, page: Math.max(1, response.total_pages) })}`, true);
          return;
        }
        setRequest({ key, result: response });
      })
      .catch((cause: unknown) => {
        if (active) setRequest({ key, error: cause instanceof Error ? cause.message : t('audit.loadError') });
      });
    return () => { active = false; controller.abort(); };
  }, [category, q, status, page, key, pathname, t]);

  const formatTimestamp = (timestamp: number) => new Date(timestamp * 1000).toLocaleString(i18n.language, { dateStyle: 'medium', timeStyle: 'medium' });
  const categoryLabel = (value: unknown) => t(`audit.categories.${isAuditCategory(value) ? value : 'other'}`);
  const statusLabel = (value: string) => value === 'success' ? t('audit.success') : value === 'failure' ? t('audit.failure') : value;

  return <div className="audit-page space-y-6 min-w-0">
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div><h1 className="text-xl font-bold tracking-tight text-[var(--ink)]">{t('audit.title')}</h1>
        <p className="mt-1 text-sm text-[var(--ink-muted)]">{t('audit.subtitle')}</p></div>
      <button type="button" className={`${control} inline-flex items-center gap-2 hover:bg-[var(--raised)] disabled:opacity-50`} disabled={loading} onClick={() => setRefresh(value => value + 1)}>
        <RefreshCw size={15} aria-hidden="true" /><span>{t('common.refresh')}</span>
      </button>
    </header>

    <section aria-labelledby="audit-category-heading" className="space-y-3">
      <h2 id="audit-category-heading" className="text-sm font-semibold">{t('audit.categoryHeading')}</h2>
      <label htmlFor="audit-category-mobile" className="sr-only">{t('audit.categoryHeading')}</label>
      <select id="audit-category-mobile" value={category} onChange={event => changeFilters({ category: event.target.value as AuditFilters['category'] })} className={`${control} w-full sm:hidden`}>
        <option value="">{t('audit.allCategories')}</option>
        {auditCategories.map(value => <option key={value} value={value}>{categoryLabel(value)}</option>)}
      </select>
      <div className="hidden flex-wrap gap-2 sm:flex" role="group" aria-labelledby="audit-category-heading">
        {(['', ...auditCategories] as const).map(value => <button key={value} type="button" aria-pressed={category === value}
          onClick={() => changeFilters({ category: value })}
          className={`${control} audit-category hover:bg-[var(--raised)]`}>
          {value ? categoryLabel(value) : t('audit.allCategories')}
        </button>)}
      </div>
      <p className="text-xs text-[var(--ink-muted)]">{t(`audit.descriptions.${category || 'all'}`)}</p>
    </section>

    <form onSubmit={event => { event.preventDefault(); changeFilters({ q: draft.trim() }); }} className="flex flex-wrap items-end gap-3 border-y border-[var(--line)] py-4">
      <div className="min-w-0 flex-1 basis-64"><label htmlFor="audit-search" className="mb-2 block text-xs font-medium">{t('audit.searchLabel')}</label>
        <div className="relative"><Search size={15} aria-hidden="true" className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
          <input id="audit-search" type="search" maxLength={200} value={draft} onChange={event => setDraft(event.target.value)} placeholder={t('audit.searchPlaceholder')} className={`${control} w-full pl-9`} />
        </div>
      </div>
      <div className="min-w-0 flex-1 basis-36 sm:flex-none"><label htmlFor="audit-status" className="mb-2 block text-xs font-medium">{t('audit.status')}</label>
        <select id="audit-status" value={status} onChange={event => changeFilters({ status: event.target.value as AuditFilters['status'] })} className={`${control} w-full`}>
          <option value="">{t('audit.filterStatus')}</option><option value="success">{t('audit.success')}</option><option value="failure">{t('audit.failure')}</option>
        </select>
      </div>
      <button type="submit" className={`${control} hover:bg-[var(--raised)]`}>{t('audit.searchButton')}</button>
      {filtersActive && <button type="button" className={`${control} hover:bg-[var(--raised)]`} onClick={resetFilters}>{t('audit.resetFilters')}</button>}
    </form>

    <section aria-labelledby="audit-results-heading" aria-busy={loading} className="space-y-3">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 id="audit-results-heading" className="text-sm font-semibold">{category ? categoryLabel(category) : t('audit.allCategories')}</h2>
        <p className="text-xs text-[var(--ink-muted)]" role="status">{loading ? t('audit.loading') : error ? t('audit.unavailable') : t('audit.resultCount', { count: result?.total ?? 0 })}</p>
      </div>
      {error ? <div role="alert" className="flex flex-wrap items-center gap-3 rounded-[var(--radius-sm)] border border-[var(--danger)]/30 p-4 text-sm">
        <AlertCircle size={18} aria-hidden="true" className="text-[var(--danger)]" /><span className="min-w-0 flex-1 break-words">{error}</span>
        <button type="button" className={control} onClick={() => setRefresh(value => value + 1)}>{t('audit.retry')}</button>
      </div> : loading ? <div className="py-12 text-center text-sm text-[var(--ink-muted)]">{t('audit.loading')}</div>
        : !result?.items.length ? <EmptyState title={t('audit.noLogs')} description={t('audit.emptyDescription')} actionLabel={filtersActive ? t('audit.resetFilters') : undefined} onAction={filtersActive ? resetFilters : undefined} />
        : <div className="rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] overflow-hidden">
          <div className="overflow-x-auto" tabIndex={0} role="region" aria-label={t('audit.resultsTable')}>
            <table className="w-full text-left text-xs">
              <caption className="sr-only">{t('audit.resultsTable')}</caption>
              <thead className="border-b border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)]"><tr>
                {['timestamp', 'actor', 'category', 'action', 'target', 'ip', 'status', 'details'].map(column => <th key={column} scope="col" className="px-4 py-3 font-medium whitespace-nowrap">{t(`audit.${column}`)}</th>)}
              </tr></thead>
              <tbody className="divide-y divide-[var(--line)]">{result.items.map(item => <tr key={item.id} className="hover:bg-[var(--raised)]/40">
                <td className="px-4 py-3 whitespace-nowrap text-[var(--ink-muted)]">{formatTimestamp(item.created_at)}</td>
                <td className="px-4 py-3 font-medium">{item.actor_username}</td>
                <td className="px-4 py-3 whitespace-nowrap">{categoryLabel(item.category)}</td>
                <td className="px-4 py-3 font-mono text-[11px] whitespace-nowrap text-[var(--primary)]">{item.action}</td>
                <td className="px-4 py-3 max-w-64 truncate" title={item.target_resource}>{item.target_resource}</td>
                <td className="px-4 py-3 whitespace-nowrap text-[var(--ink-muted)]">{item.ip_address || '—'}</td>
                <td className="px-4 py-3 whitespace-nowrap"><StatusPill label={statusLabel(item.status)} variant={item.status === 'success' ? 'success' : item.status === 'failure' ? 'danger' : 'neutral'} /></td>
                <td className="px-4 py-1"><button type="button" aria-label={t('audit.inspect', { id: item.id })} className={`${control} inline-flex items-center justify-center hover:bg-[var(--raised)]`} onClick={() => setInspectLog(item)}><Eye size={16} aria-hidden="true" /></button></td>
              </tr>)}</tbody>
            </table>
          </div>
          {result.total_pages > 1 && <nav aria-label={t('audit.pagination')} className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--line)] px-4 py-3 text-xs">
            <span>{t('audit.pageOf', { page, total: result.total_pages })}</span>
            <div className="flex gap-2"><button type="button" className={`${control} disabled:opacity-40`} aria-label={t('audit.previousPage')} disabled={page <= 1} onClick={() => changeFilters({ page: page - 1 })}><ChevronLeft size={16} aria-hidden="true" /></button>
              <button type="button" className={`${control} disabled:opacity-40`} aria-label={t('audit.nextPage')} disabled={page >= result.total_pages} onClick={() => changeFilters({ page: page + 1 })}><ChevronRight size={16} aria-hidden="true" /></button></div>
          </nav>}
        </div>}
    </section>

    {inspectLog && <ModalOverlay onClick={() => setInspectLog(null)} onDismiss={() => setInspectLog(null)} className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50">
      <div className="audit-detail dialog-panel w-full max-w-2xl space-y-4 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5" onClick={event => event.stopPropagation()}>
        <header className="dialog-header flex items-start justify-between gap-3"><h2 className="text-base font-semibold">{t('audit.detailTitle', { id: inspectLog.id })}</h2><button type="button" className={control} aria-label={t('audit.closeDetail')} onClick={() => setInspectLog(null)}><X size={18} aria-hidden="true" /></button></header>
        <div className="dialog-body space-y-4">
          <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2 text-xs">
            {([
              ['category', categoryLabel(inspectLog.category)], ['actor', inspectLog.actor_username], ['action', inspectLog.action], ['status', statusLabel(inspectLog.status)], ['target', inspectLog.target_resource], ['ip', inspectLog.ip_address || '—'], ['timestamp', formatTimestamp(inspectLog.created_at)],
            ] as const).map(([label, value]) => <div key={label} className="min-w-0"><dt className="text-[var(--ink-muted)]">{t(`audit.${label}`)}</dt><dd className="mt-1 break-words font-medium">{value}</dd></div>)}
          </dl>
          <div><h3 className="mb-2 text-xs font-medium">{t('audit.details')}</h3><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] p-3 font-mono text-xs">{auditDetails(inspectLog.details_json) ?? t('audit.noDetails')}</pre></div>
        </div>
      </div>
    </ModalOverlay>}
  </div>;
}
