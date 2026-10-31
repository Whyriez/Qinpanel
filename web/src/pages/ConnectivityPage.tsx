import { useTranslation } from 'react-i18next';
import { ConnectivitySettings } from '../components/system/ConnectivitySettings';
import { useEffect, useState } from 'react';
import { ArrowRight, Globe } from 'lucide-react';
import { useAuth } from '../context/AuthContext';
import { fetchSites } from '../api/client';
import type { Site } from '../types/api';
import { RouteLink } from '../lib/router';
import { sitePath } from '../lib/siteRoutes';

export function ConnectivityPage() {
  const { user } = useAuth();
  return user?.role === 'admin' ? <AdminConnectivityPage /> : <ProjectConnectionPicker key={user?.id} />;
}
function AdminConnectivityPage() {
  const { i18n } = useTranslation();
  return <div className="space-y-6"><h1 className="text-xl font-bold">{i18n.language.startsWith('en') ? 'Connectivity' : 'Konektivitas'}</h1><ConnectivitySettings /></div>;
}
function ProjectConnectionPicker() {
  const { t, i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const [sites, setSites] = useState<Site[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let active = true;
    setLoading(true); setError(''); setSites([]);
    fetchSites().then(result => { if (active) setSites(result); })
      .catch(cause => { if (active) setError(cause instanceof Error ? cause.message : t('common.error')); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [reload, t]);
  return <div className="space-y-5">
    <div><h1 className="text-xl font-bold">{t('sites.tabs.connections')}</h1><p className="mt-1 text-sm text-[var(--ink-muted)]">{en ? 'Select a project to view its connection information.' : 'Pilih project untuk melihat informasi koneksinya.'}</p></div>
    {loading && <p role="status">{t('common.loading')}</p>}
    {error && <div className="space-y-3"><p role="alert" className="text-sm text-[var(--danger)]">{error}</p><button className="button-secondary" onClick={() => setReload(value => value + 1)}>{t('common.refresh')}</button></div>}
    {!loading && !error && !sites.length && <p className="text-sm text-[var(--ink-muted)]">{en ? 'No projects yet. Connection information will appear in each project.' : 'Belum ada project. Informasi koneksi akan tersedia di detail masing-masing project.'}</p>}
    <div className="grid gap-3 sm:grid-cols-2">{sites.map(site => <RouteLink key={site.id} href={sitePath(site, 'connections')} className="flex items-center gap-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-4 text-sm hover:border-[var(--primary)]"><Globe size={18} aria-hidden="true" /><span className="min-w-0 flex-1 break-all font-medium">{site.primary_domain}</span><ArrowRight size={16} aria-hidden="true" /></RouteLink>)}</div>
  </div>;
}
