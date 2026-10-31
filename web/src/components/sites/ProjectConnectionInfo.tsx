import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowRight, Copy, Database, HardDrive, Mail, RefreshCw, Terminal } from 'lucide-react';
import { fetchConnectivity, endpointAuthority, serviceName, transportName, type ConnectivityOverview, type Endpoint } from '../../api/connectivity';
import { RouteLink } from '../../lib/router';
import { sitePath } from '../../lib/siteRoutes';
import type { Site } from '../../types/api';

export function ConnectionAddress({ endpoint: e }: { endpoint: Endpoint }) {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const [copied, setCopied] = useState('');
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(endpointAuthority(e));
      setCopied(en ? 'Address copied.' : 'Alamat disalin.');
    } catch {
      setCopied(en ? 'Copy failed. Select the address text to copy it.' : 'Gagal menyalin. Pilih teks alamat untuk menyalinnya.');
    }
  };
  return <article className="rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-4 space-y-3">
    <div className="flex flex-wrap justify-between gap-2">
      <h3 className="font-semibold text-sm">{serviceName(e.service, en)}</h3>
      <span className="text-xs text-[var(--ink-muted)]">{!e.enabled ? (en ? 'Disabled' : 'Dinonaktifkan') : e.primary ? (en ? 'Primary' : 'Utama') : (en ? 'Alternative' : 'Alternatif')}</span>
    </div>
    <div className="flex items-start justify-between gap-3">
      <code className="break-all select-all text-sm">{endpointAuthority(e)}</code>
      <button type="button" className="button-secondary shrink-0" onClick={() => void copy()} aria-label={`${en ? 'Copy address' : 'Salin alamat'} ${e.host}`}><Copy size={16} aria-hidden="true" /></button>
    </div>
    <dl className="grid grid-cols-2 gap-3 text-xs">
      <div><dt className="text-[var(--ink-muted)]">{en ? 'Connection route' : 'Jalur koneksi'}</dt><dd className="mt-1">{transportName(e.transport, en)}</dd></div>
      <div><dt className="text-[var(--ink-muted)]">{en ? 'Encryption' : 'Enkripsi'}</dt><dd className="mt-1">{e.tls_mode === 'none' ? (en ? 'No TLS' : 'Tanpa TLS') : e.tls_mode.toUpperCase()}</dd></div>
    </dl>
    {e.client_requirement !== 'none' && <p className="text-xs text-[var(--ink-muted)]">{e.client_requirement === 'cloudflared' ? (en ? 'Requires cloudflared on your device.' : 'Memerlukan cloudflared pada perangkat Anda.') : (en ? 'Requires an organization-enrolled Cloudflare One Client.' : 'Memerlukan Cloudflare One Client yang terdaftar pada organisasi.')}</p>}
    {copied && <p role="status" className="text-xs">{copied}</p>}
  </article>;
}

export function ProjectConnectionInfo({ site, onOpenSsh }: { site: Site; onOpenSsh: () => void }) {
  const { t, i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const [page, setPage] = useState(1);
  const [reload, setReload] = useState(0);
  const [data, setData] = useState<ConnectivityOverview | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    setLoading(true); setError(''); setData(null);
    fetchConnectivity(page, '', site.id).then(result => {
      if (!active) return;
      if (page > Math.max(1, result.total_pages)) { setPage(Math.max(1, result.total_pages)); return; }
      setData(result);
    }).catch(cause => {
      if (active) setError(cause instanceof Error ? cause.message : t('common.error'));
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [site.id, page, reload, t]);
  return <section className="space-y-6" aria-labelledby="project-connections-title">
    <div>
      <h2 id="project-connections-title" className="text-lg font-semibold">{t('sites.tabs.connections')}</h2>
      <p className="mt-1 text-sm text-[var(--ink-muted)]">{en ? 'Find connection guides and registered addresses for this project.' : 'Lihat panduan koneksi dan alamat layanan yang terdaftar untuk project ini.'}</p>
    </div>
    <div className="grid gap-3 sm:grid-cols-2">
      <button type="button" className="button-secondary justify-between p-4" onClick={onOpenSsh}><span className="flex items-center gap-2"><Terminal size={17} aria-hidden="true" />{en ? 'SSH / SFTP guide' : 'Panduan SSH / SFTP'}</span><ArrowRight size={16} aria-hidden="true" /></button>
      <RouteLink className="button-secondary justify-between p-4" href={sitePath(site, 'database')}><span className="flex items-center gap-2"><Database size={17} aria-hidden="true" />{en ? 'Project databases' : 'Database project'}</span><ArrowRight size={16} aria-hidden="true" /></RouteLink>
      <RouteLink className="button-secondary justify-between p-4" href={sitePath(site, 'storage')}><span className="flex items-center gap-2"><HardDrive size={17} aria-hidden="true" />{en ? 'S3 storage connections' : 'Koneksi storage S3'}</span><ArrowRight size={16} aria-hidden="true" /></RouteLink>
      <RouteLink className="button-secondary justify-between p-4" href="/email"><span className="flex items-center gap-2"><Mail size={17} aria-hidden="true" />{en ? 'Email accounts' : 'Akun email'}</span><ArrowRight size={16} aria-hidden="true" /></RouteLink>
    </div>
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">{en ? 'Registered service addresses' : 'Alamat layanan terdaftar'}</h3>
        <button type="button" className="button-secondary" disabled={loading} onClick={() => setReload(value => value + 1)}><RefreshCw size={14} aria-hidden="true" />{t('common.refresh')}</button>
      </div>
      <p className="text-xs text-[var(--ink-muted)]">{en ? 'Addresses are managed by your administrator. Use the service guides for credentials and connection instructions.' : 'Alamat dikelola administrator. Gunakan panduan layanan untuk kredensial dan langkah koneksi.'}</p>
      {loading && <p role="status" className="text-sm text-[var(--ink-muted)]">{t('common.loading')}</p>}
      {error && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
      {data && !data.total && <div className="rounded-[var(--radius-md)] border border-[var(--line)] p-5 text-sm text-[var(--ink-muted)]">{en ? 'No service addresses have been registered for this project. Use the guides above or contact your administrator.' : 'Belum ada alamat layanan yang didaftarkan untuk project ini. Gunakan panduan di atas atau hubungi administrator.'}</div>}
      {data && <div className="grid gap-4 md:grid-cols-2">{data.endpoints.map(({ endpoint }) => <ConnectionAddress key={endpoint.id} endpoint={endpoint} />)}</div>}
      {data && data.total_pages > 1 && <nav className="flex flex-wrap items-center gap-3 text-sm" aria-label={en ? 'Service address pages' : 'Halaman alamat layanan'}>
        <button type="button" className="button-secondary" disabled={page <= 1} onClick={() => setPage(value => value - 1)}>{en ? 'Previous' : 'Sebelumnya'}</button>
        <span>{page} / {data.total_pages}</span>
        <button type="button" className="button-secondary" disabled={page >= data.total_pages} onClick={() => setPage(value => value + 1)}>{en ? 'Next' : 'Berikutnya'}</button>
      </nav>}
    </div>
  </section>;
}
