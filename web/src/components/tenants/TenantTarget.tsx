import React, { useEffect, useId, useState } from 'react';
import { ArrowRight, Loader2, RefreshCw, Users } from 'lucide-react';
import { useAuth } from '../../context/AuthContext';
import { fetchTenants } from '../../api/client';
import type { TenantListItem } from '../../types/api';
import { navigate, RouteLink, useBrowserRoute } from '../../lib/router';

export function TenantTargetNotice({ state, error, onRetry }: {
  state: 'loading' | 'error' | 'empty' | 'choose'; error?: string; onRetry: () => void;
}) {
  return <div className="tenant-target-notice" role={state === 'error' ? 'alert' : 'status'}>
    <div className="tenant-target-symbol" aria-hidden="true">{state === 'loading' ? <Loader2 size={22} className="animate-spin" /> : <Users size={22} />}</div>
    <h2>{state === 'loading' ? 'Memuat daftar tenant' : state === 'error' ? 'Daftar tenant belum dapat dimuat' : state === 'empty' ? 'Belum ada tenant yang dapat dikelola' : 'Pilih tenant untuk melanjutkan'}</h2>
    <p>{state === 'loading' ? 'Menyiapkan pilihan akun hosting yang dapat Anda kelola.' : state === 'error' ? error : state === 'empty' ? 'Tambahkan tenant terlebih dahulu. Setelah akun hosting siap, Anda dapat mengelola berkas dan membuka terminal sesuai izin aksesnya.' : 'Pilih akun hosting pada daftar di atas untuk membuka ruang kerjanya.'}</p>
    {state === 'empty' && <RouteLink href="/tenants" className="access-button access-button-primary">Kelola tenant <ArrowRight size={16} aria-hidden="true" /></RouteLink>}
    {state === 'error' && <button type="button" className="access-button" onClick={onRetry}><RefreshCw size={16} aria-hidden="true" />Coba lagi</button>}
  </div>;
}

export function TenantTarget({ children }: { children: (tenant?: string) => React.ReactNode }) {
  const { user } = useAuth();
  const route = useBrowserRoute();
  const id = useId();
  const [tenants, setTenants] = useState<TenantListItem[] | null>(null);
  const [error, setError] = useState('');
  const [revision, setRevision] = useState(0);
  const admin = user?.role === 'admin';
  const retry = () => setRevision(value => value + 1);
  useEffect(() => {
    let active = true;
    if (admin) {
      setTenants(null);
      setError('');
      fetchTenants().then(rows => {
        if (active) setTenants(rows.filter(tenant => tenant.status === 'active' && !!tenant.linux_username && !!tenant.linux_uid));
      }).catch((cause: unknown) => {
        if (active) setError(cause instanceof Error ? cause.message : 'Periksa koneksi Anda, lalu coba lagi.');
      });
    }
    return () => { active = false; };
  }, [admin, revision]);
  if (!admin) return <>{children()}</>;
  if (error) return <TenantTargetNotice state="error" error={error} onRetry={retry} />;
  if (!tenants) return <TenantTargetNotice state="loading" onRetry={retry} />;
  if (tenants.length === 0) return <TenantTargetNotice state="empty" onRetry={retry} />;
  const selected = tenants.find(tenant => String(tenant.id) === route.search.get('tenant'));
  return <div className="tenant-target"><div className="tenant-target-toolbar">
    <div><label htmlFor={id}>Akun tenant</label><p>Kelola akses dan ruang kerja untuk akun yang dipilih.</p></div>
    <select id={id} value={selected?.id || ''} onChange={event => {
      const search = new URLSearchParams(route.search);
      if (event.target.value) search.set('tenant', event.target.value); else search.delete('tenant');
      navigate(`${route.pathname}${search.size ? `?${search}` : ''}`);
    }}><option value="">Pilih tenant</option>{tenants.map(tenant => <option key={tenant.id} value={tenant.id}>{tenant.username}</option>)}</select>
    <RouteLink href="/tenants" className="access-button">Kelola tenant <ArrowRight size={15} aria-hidden="true" /></RouteLink>
  </div>{selected ? children(String(selected.id)) : <TenantTargetNotice state="choose" onRetry={retry} />}</div>;
}
