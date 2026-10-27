import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, Sparkles, X } from 'lucide-react';
import { fetchAiSettings, recommendAiPackage, validAiDraft, type AiRecommendation, type AiSettings } from '../../api/ai';
import type { PackageDemand } from '../../types/api';
import { ModalOverlay } from '../ui/ModalOverlay';
import '../../styles/ai.css';

const field = 'mt-1 min-h-11 w-full min-w-0 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-sm text-[var(--ink)]';
export function AiPackageAssistant({ onClose, onUse }: { onClose: () => void; onUse: (result: AiRecommendation) => void }) {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const text = (id: string, english: string) => en ? english : id;
  const id = useId();
  const controller = useRef<AbortController | null>(null);
  const preview = useRef<HTMLElement | null>(null);
  const [settings, setSettings] = useState<AiSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [description, setDescription] = useState('');
  const [demand, setDemand] = useState<PackageDemand>({ usage: 'personal', projects_per_tenant: 1, tenant_slots: 1 });
  const [consent, setConsent] = useState(false);
  const [result, setResult] = useState<AiRecommendation | null>(null);
  const [error, setError] = useState('');
  const [now, setNow] = useState(Date.now() / 1000);
  const load = async () => {
    controller.current?.abort(); const current = new AbortController(); controller.current = current;
    setLoading(true); setError('');
    try { const data = await fetchAiSettings(current.signal); if (!current.signal.aborted) setSettings(data); }
    catch (e) { if (!current.signal.aborted) setError(e instanceof Error ? e.message : 'Gagal memuat integrasi AI.'); }
    finally { if (!current.signal.aborted) setLoading(false); }
  };
  useEffect(() => { void load(); return () => controller.current?.abort(); }, []);
  useEffect(() => {
    if (!result) return;
    preview.current?.focus({ preventScroll: true });
    preview.current?.scrollIntoView({ block: 'start' });
    const timer = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(timer);
  }, [result]);
  const invalidate = () => { setResult(null); setError(''); };
  const analyze = async () => {
    if (!settings || !consent || !settings.enabled) return;
    controller.current?.abort(); const current = new AbortController(); controller.current = current;
    setBusy(true); invalidate();
    try {
      const data = await recommendAiPackage(description, demand, settings.revision, en, current.signal);
      if (!current.signal.aborted) { setResult(data); setNow(Date.now() / 1000); }
    } catch (e) { if (!current.signal.aborted) setError(e instanceof Error ? e.message : 'Analisis AI gagal.'); }
    finally { if (!current.signal.aborted) setBusy(false); }
  };
  const useDraft = async () => {
    if (!result || !validAiDraft(result)) { setResult(null); setError(text('Preview kedaluwarsa. Buat analisis baru.', 'Preview expired. Run a new analysis.')); return; }
    controller.current?.abort(); const current = new AbortController(); controller.current = current;
    setBusy(true); setError('');
    try {
      const currentSettings = await fetchAiSettings(current.signal);
      if (current.signal.aborted) return;
      if (!currentSettings.enabled || currentSettings.revision !== result.settings_revision || currentSettings.provider !== result.provider || !validAiDraft(result)) {
        setSettings(currentSettings); setConsent(false); setResult(null);
        throw new Error(text('Pengaturan atau preview berubah. Tinjau kembali lalu buat analisis baru.', 'Settings or preview changed. Review and run a new analysis.'));
      }
      onUse(result);
    } catch (e) { if (!current.signal.aborted) setError(e instanceof Error ? e.message : 'Gagal menggunakan draft.'); }
    finally { if (!current.signal.aborted) setBusy(false); }
  };
  const configured = settings?.enabled && settings.providers.find(p => p.id === settings.provider)?.has_key;
  const remaining = result ? Math.max(0, Math.ceil(result.preset.expires_at - now)) : 0;
  const profile = result?.profile === 'performance' ? text('Performa', 'Performance') : result?.profile === 'conservative' ? text('Konservatif', 'Conservative') : text('Seimbang', 'Balanced');
  return <ModalOverlay onDismiss={onClose} aria-labelledby={`${id}-title`}>
    <div className="ai-package-assistant dialog-panel w-full max-w-2xl min-w-0 border border-[var(--line)] bg-[var(--surface)] p-4 sm:p-6" data-testid="ai-package-assistant">
      <header className="dialog-header flex items-start justify-between gap-3"><div><h2 id={`${id}-title`} className="flex items-center gap-2 font-semibold"><Sparkles size={18} aria-hidden="true" />{text('Bantu atur dengan AI', 'Help size with AI')}</h2>
        <p className="mt-2 text-sm text-[var(--ink-muted)]">{text('Ceritakan kebutuhan Anda untuk menyiapkan draft paket baru.', 'Describe your needs to prepare a new hosting plan draft.')}</p></div>
        <button type="button" className="button-secondary min-h-11 min-w-11 shrink-0" aria-label={text('Tutup asisten AI', 'Close AI assistant')} onClick={onClose}><X size={16} aria-hidden="true" /></button></header>
      <div className="dialog-body min-w-0 space-y-5 pt-4">
        {loading ? <p role="status" className="flex gap-2 text-sm"><Loader2 size={16} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />{text('Memuat integrasi AI…', 'Loading AI integration…')}</p> : !configured ? <div className="space-y-3 text-sm"><p>{text('Aktifkan integrasi dan simpan API key sebelum menggunakan asisten.', 'Enable the integration and save an API key before using the assistant.')}</p><a className="button-primary inline-flex" href="/settings?section=ai">{text('Atur integrasi AI', 'Configure AI integration')}</a></div> : <form className="space-y-4" onSubmit={event => { event.preventDefault(); void analyze(); }}>
          <p className="text-xs text-[var(--ink-muted)]">{settings?.provider === 'openai' ? 'OpenAI' : 'Gemini'} · {settings?.model}</p>
          <div><label htmlFor={`${id}-description`} className="text-sm font-medium">{text('Kebutuhan aplikasi', 'Application needs')}</label><textarea id={`${id}-description`} className={`${field} min-h-28 resize-y`} required maxLength={2000} value={description} disabled={busy} placeholder={text('Contoh: server pribadi untuk dua aplikasi Laravel dan satu database. Prioritaskan performa dengan cadangan memori yang cukup.', 'Example: a personal server for two Laravel apps and one database. Prioritize performance with adequate memory headroom.')} onChange={e => { setDescription(e.target.value); invalidate(); }} /></div>
          <div><label htmlFor={`${id}-usage`} className="text-sm">{text('Penggunaan server', 'Server use')}</label><select id={`${id}-usage`} className={field} value={demand.usage} disabled={busy} onChange={e => { const usage = e.target.value as PackageDemand['usage']; setDemand({ usage, projects_per_tenant: usage === 'multi_project' ? 2 : 1, tenant_slots: usage === 'multi_tenant' ? 2 : 1 }); invalidate(); }}>
            <option value="personal">{text('Pribadi · satu project', 'Personal · one project')}</option><option value="multi_project">{text('Beberapa project sendiri', 'Multiple personal projects')}</option><option value="multi_tenant">{text('Hosting beberapa tenant', 'Multiple hosting tenants')}</option>
          </select></div>
          {demand.usage !== 'personal' && <div className="grid gap-4 sm:grid-cols-2"><div><label htmlFor={`${id}-projects`} className="text-sm">{text('Project per tenant', 'Projects per tenant')}</label><input id={`${id}-projects`} className={field} type="number" min={demand.usage === 'multi_project' ? 2 : 1} max={256} required value={demand.projects_per_tenant} disabled={busy} onChange={e => { setDemand(v => ({ ...v, projects_per_tenant: Number(e.target.value) })); invalidate(); }} /></div>
            {demand.usage === 'multi_tenant' && <div><label htmlFor={`${id}-tenants`} className="text-sm">{text('Jumlah tenant', 'Tenant slots')}</label><input id={`${id}-tenants`} className={field} type="number" min={2} max={10000} required value={demand.tenant_slots} disabled={busy} onChange={e => { setDemand(v => ({ ...v, tenant_slots: Number(e.target.value) })); invalidate(); }} /></div>}</div>}
          <p className="text-xs leading-relaxed text-[var(--ink-muted)]">{text('Jumlah di atas menjadi acuan pembagian kapasitas. AI memilih profil; batas resource dihitung server dari kapasitas tersedia, bukan dibuat oleh model.', 'These counts determine capacity shares. AI selects a profile; the server computes resource limits from available capacity.')}</p>
          <label className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-1 size-4 shrink-0" required checked={consent} disabled={busy} onChange={e => { setConsent(e.target.checked); invalidate(); }} /><span>{text('Saya setuju mengirim teks kebutuhan, jumlah project/tenant, dan ringkasan kapasitas ke provider di atas. Jangan sertakan password, API key, source code atau isi .env. Permintaan dapat dikenai biaya.', 'I agree to send my description, project/tenant counts and capacity summary to the provider above. Do not include passwords, API keys, source code or .env contents. Requests may incur charges.')}</span></label>
          <button type="submit" className="button-primary min-h-11" disabled={busy || !consent || !description.trim()}>{busy ? <span role="status" className="inline-flex items-center gap-2"><Loader2 size={15} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />{text('Menganalisis kebutuhan…', 'Analyzing needs…')}</span> : text('Analisis kebutuhan', 'Analyze needs')}</button>
        </form>}
        {error && <div role="alert" className="space-y-2 text-sm text-[var(--danger)]"><p>{error}</p>{!settings && <button type="button" className="button-secondary min-h-11" onClick={() => void load()}>{text('Coba lagi', 'Retry')}</button>}</div>}
        {result && <section ref={preview} tabIndex={-1} className="space-y-3 border-t border-[var(--line)] pt-4" aria-label={text('Preview paket AI', 'AI plan preview')}>
          <h3 className="text-sm font-semibold">{text('Profil yang disarankan', 'Suggested profile')}: {profile}</h3><p className="whitespace-pre-wrap break-words text-sm">{result.summary}</p>
          <dl className="grid grid-cols-2 gap-3 rounded-[var(--radius-sm)] bg-[var(--surface-sunken)] p-3 text-sm">
            <div><dt className="text-xs text-[var(--ink-muted)]">CPU</dt><dd>{result.preset.cpu_quota_percent}%</dd></div><div><dt className="text-xs text-[var(--ink-muted)]">RAM</dt><dd>{Math.floor(result.preset.memory_limit_bytes / 1048576)} MiB</dd></div>
            <div><dt className="text-xs text-[var(--ink-muted)]">Disk</dt><dd>{Math.floor(result.preset.disk_limit_bytes / 1048576)} MiB</dd></div><div><dt className="text-xs text-[var(--ink-muted)]">{text('Situs', 'Sites')}</dt><dd>{result.preset.max_sites}</dd></div>
          </dl>
          {result.considerations.length > 0 && <ul className="list-disc space-y-1 pl-5 text-xs text-[var(--ink-muted)]">{result.considerations.map((item, index) => <li key={index} className="break-words">{item}</li>)}</ul>}
          <p className="text-xs text-[var(--ink-muted)]">{text('CPU 100% setara satu core. Snapshot sesaat bukan reservasi kapasitas atau pengukuran puncak beban. Formulir tetap dapat diedit; tenant existing tidak diubah.', '100% CPU equals one core. A snapshot is neither a capacity reservation nor a peak-load measurement. The form remains editable; existing tenants are unchanged.')}</p>
          <p className={`text-xs ${remaining ? 'text-[var(--ink-muted)]' : 'text-[var(--danger)]'}`}>{remaining ? text(`Preview berlaku ${remaining} detik lagi.`, `Preview valid for ${remaining} more seconds.`) : text('Preview kedaluwarsa. Jalankan analisis lagi.', 'Preview expired. Run the analysis again.')}</p>
          <button type="button" className="button-primary min-h-11" disabled={busy || !remaining} onClick={() => void useDraft()}>{text('Isi formulir paket baru', 'Fill new plan form')}</button>
        </section>}
      </div>
      <footer className="dialog-footer flex justify-end border-t border-[var(--line)] pt-4"><button type="button" className="button-secondary min-h-11" onClick={onClose}>{text('Tutup', 'Close')}</button></footer>
    </div>
  </ModalOverlay>;
}
