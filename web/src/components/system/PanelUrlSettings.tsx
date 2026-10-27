import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { request } from '../../api/client';
import { endpointAuthority, type Endpoint } from '../../api/connectivity';
import { ModalOverlay } from '../ui/ModalOverlay';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';

export interface PanelUrlPolicy { revision: number; canonical_url: string | null; current_url: string; endpoint_id: string | null; endpoint_revision: number | null; aliases: string[] }
export const panelEndpointUrl = (e: Endpoint) => `${e.tls_mode === 'tls' ? 'https' : 'http'}://${endpointAuthority(e)}`;
export const samePanelAddress = (left: string, right: string) => { try { return new URL(left).origin === new URL(right).origin; } catch { return false; } };
export function PanelUrlSettings({ endpoints, disabled = false, onReloadEndpoints }: { endpoints: Endpoint[]; disabled?: boolean; onReloadEndpoints?: () => void }) {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const text = (id: string, english: string) => en ? english : id;
  const [policy, setPolicy] = useState<PanelUrlPolicy | null>(null);
  const [error, setError] = useState(''), [success, setSuccess] = useState('');
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<Endpoint | null>(null), [confirmed, setConfirmed] = useState(false);
  const [revoking, setRevoking] = useState('');
  const active = useRef(false), pending = useRef(false), sequence = useRef(0);
  const load = async () => {
    const seq = ++sequence.current; setLoading(true); setError('');
    try { const result = await request<PanelUrlPolicy>('/api/v1/connectivity/panel-url'); if (active.current && seq === sequence.current) setPolicy(result); }
    catch (error) { if (active.current && seq === sequence.current) { setPolicy(null); setError(error instanceof Error ? error.message : 'Pengaturan URL gagal dimuat.'); } }
    finally { if (active.current && seq === sequence.current) setLoading(false); }
  };
  useEffect(() => { active.current = true; void load(); return () => { active.current = false; ++sequence.current; }; }, []);
  const activate = async () => {
    if (!selected || !policy || !confirmed || pending.current) return;
    pending.current = true; setBusy(true); setError(''); setSuccess('');
    try {
      const next = await request<PanelUrlPolicy>('/api/v1/connectivity/panel-url/activate', { method: 'POST', body: JSON.stringify({ endpoint_id: selected.id, expected_endpoint_revision: selected.revision, expected_revision: policy.revision, reachable_confirmed: true }) });
      if (active.current) { setPolicy(next); setSelected(null); setSuccess(text('URL panel diaktifkan. Alamat sebelumnya tetap tersedia sampai Anda mencabutnya.', 'Panel URL activated. Previous addresses remain available until revoked.')); }
    } catch (error) { if (active.current) setError(error instanceof Error ? error.message : 'URL panel gagal diaktifkan.'); }
    finally { pending.current = false; if (active.current) setBusy(false); }
  };
  const revoke = async () => {
    if (!revoking || !policy || pending.current) return;
    pending.current = true; setBusy(true); setError(''); setSuccess('');
    try {
      const next = await request<PanelUrlPolicy>('/api/v1/connectivity/panel-url/revoke', { method: 'POST', body: JSON.stringify({ origin: revoking, expected_revision: policy.revision }) });
      if (active.current) { setPolicy(next); setRevoking(''); setSuccess(text('Akses melalui alamat transisi dicabut.', 'Access through the transition address was revoked.')); }
    } catch (error) { if (active.current) { setRevoking(''); setError(error instanceof Error ? error.message : 'Alamat gagal dicabut.'); } }
    finally { pending.current = false; if (active.current) setBusy(false); }
  };
  const candidates = endpoints.filter(e => e.service === 'panel' && e.scope.kind === 'host' && e.enabled && e.primary);
  return <section className="space-y-3 border-b border-[var(--line)] pb-5" aria-label={text('URL utama panel', 'Primary panel URL')}>
    <h3 className="font-semibold text-sm">{text('URL utama panel', 'Primary panel URL')}</h3>
    <p className="text-sm text-[var(--ink-muted)]">{text('Pilih URL untuk tautan email dan alamat panel yang diizinkan. Penyimpanan alamat di bawah tidak langsung mengaktifkan URL atau mengubah proxy, DNS, dan sertifikat.', 'Choose the URL for email links and allowed panel addresses. Saving an address below does not activate its URL or change proxies, DNS, or certificates.')}</p>
    {loading ? <p role="status" className="text-sm">{text('Memuat URL panel…', 'Loading panel URL…')}</p> : policy && <>
      <p className="text-sm">{text('URL aktif', 'Active URL')}: <code className="break-all">{policy.canonical_url || text('Belum diaktifkan', 'Not activated')}</code></p>
      <p className="text-xs text-[var(--ink-muted)]">{text('Alamat sesi ini', 'Current session address')}: <code className="break-all">{policy.current_url}</code></p>
      {!policy.canonical_url && <p className="text-xs">{text('Email belum menyertakan tautan absolut sampai URL diaktifkan.', 'Emails will not contain an absolute link until a URL is activated.')}</p>}
      {candidates.map(e => <button key={e.id} type="button" className="button-secondary" disabled={disabled || busy || (policy.endpoint_id === e.id && policy.endpoint_revision === e.revision)} onClick={() => { setSelected(e); setConfirmed(false); setError(''); }}>{text('Aktifkan URL utama', 'Activate primary URL')}<span className="sr-only"> {e.host}</span></button>)}
      {!candidates.length && <p className="text-xs text-[var(--ink-muted)]">{text('Gunakan filter Panel untuk melihat alamat utama yang dapat diaktifkan. Tambahkan alamat Panel jika belum ada.', 'Use the Panel filter to see a primary address that can be activated. Add a Panel address if none exists.')}</p>}
      {policy.aliases.length > 0 && <div className="space-y-2"><p className="text-sm font-medium">{text('Alamat transisi', 'Transition addresses')}</p>{policy.aliases.map(origin => <div key={origin} className="flex flex-wrap justify-between items-center gap-2 text-xs"><code className="break-all">{origin}</code><button type="button" className="button-secondary" disabled={disabled || busy || samePanelAddress(origin, policy.current_url)} onClick={() => setRevoking(origin)}>{text('Cabut akses alamat', 'Revoke address access')}<span className="sr-only"> {origin}</span></button></div>)}<p className="text-xs text-[var(--ink-muted)]">{text('Masuk melalui URL aktif sebelum mencabut alamat lama. Sesi browser tetap terpisah antar hostname.', 'Sign in through the active URL before revoking an old address. Browser sessions remain separate across hostnames.')}</p></div>}
    </>}
    {success && <p role="status" className="text-sm">{success}</p>}
    {error && !selected && <div role="alert" className="text-sm text-[var(--danger)] space-y-2"><p>{error}</p><button type="button" className="button-secondary" disabled={busy || disabled} onClick={() => { setSelected(null); onReloadEndpoints?.(); void load(); }}>{text('Muat ulang pengaturan URL', 'Reload URL settings')}</button></div>}
    {selected && policy && <ModalOverlay onDismiss={() => setSelected(null)} dismissDisabled={busy} className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" aria-labelledby="activate-panel-url-title"><div className="dialog-panel w-full max-w-xl space-y-4 border border-[var(--line)] rounded-[var(--radius-lg)] bg-[var(--surface)] p-5"><h3 id="activate-panel-url-title" className="font-semibold">{text('Aktifkan URL utama panel', 'Activate the primary panel URL')}</h3><div className="dialog-body space-y-3 text-sm"><p>{text('URL baru', 'New URL')}: <code className="break-all">{panelEndpointUrl(selected)}</code></p><p>{text('Alamat yang sedang Anda pakai dan URL sebelumnya akan dipertahankan selama transisi.', 'Your current address and previous URLs will be preserved during the transition.')}</p><a className="underline" href={`${panelEndpointUrl(selected)}/connectivity/verify`} target="_blank" rel="noopener noreferrer">{text('Buka pemeriksaan akses di tab baru', 'Open the access check in a new tab')}</a><label className="flex items-start gap-2"><input className="mt-0.5 size-4 shrink-0" type="checkbox" disabled={busy} checked={confirmed} onChange={e => setConfirmed(e.target.checked)} /><span>{text('Saya berhasil membuka pemeriksaan melalui URL baru.', 'I successfully opened the check through the new URL.')}</span></label><p className="text-xs text-[var(--ink-muted)]">{text('Aktivasi ini tidak memasang sertifikat atau mengubah listener. HTTPS aktif tidak dapat diturunkan ke HTTP.', 'Activation does not install a certificate or change a listener. An active HTTPS URL cannot be downgraded to HTTP.')}</p>{error && <p role="alert">{error}</p>}</div><div className="flex flex-wrap justify-end gap-2"><button className="button-secondary" type="button" disabled={busy} onClick={() => setSelected(null)}>{text('Batal', 'Cancel')}</button><button className="button-primary" type="button" disabled={!confirmed || busy || disabled} onClick={() => void activate()}>{busy ? text('Mengaktifkan…', 'Activating…') : text('Aktifkan URL', 'Activate URL')}</button></div></div></ModalOverlay>}
    <ConfirmDangerDialog isOpen={Boolean(revoking)} onClose={() => setRevoking('')} onConfirm={() => void revoke()} isLoading={busy} title={text('Cabut akses alamat transisi', 'Revoke transition address access')} description={text('Panel akan menolak akses melalui alamat ini. DNS, proxy dan sertifikat tidak dihapus.', 'The panel will reject access through this address. DNS, proxies and certificates are not removed.')} confirmMatchText={revoking} confirmButtonLabel={text('Cabut akses', 'Revoke access')} />
  </section>;
}
