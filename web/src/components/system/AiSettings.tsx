import { useCallback, useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, PlugZap } from 'lucide-react';
import { fetchAiSettings, saveAiSettings, testAiConnection, type AiProvider, type AiSettings as Settings } from '../../api/ai';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';
import '../../styles/ai.css';

const field = 'mt-1 min-h-11 w-full min-w-0 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-sm text-[var(--ink)]';
export function AiSettings() {
  const { i18n } = useTranslation();
  const text = (id: string, en: string) => i18n.language.startsWith('en') ? en : id;
  const id = useId();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [provider, setProvider] = useState<AiProvider>('gemini');
  const [model, setModel] = useState('');
  const [enabled, setEnabled] = useState(false);
  const [limit, setLimit] = useState(20);
  const [key, setKey] = useState('');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [consent, setConsent] = useState(false);
  const [removing, setRemoving] = useState(false);
  const load = useCallback(async (signal?: AbortSignal) => {
    setLoading(true); setError('');
    try {
      const data = await fetchAiSettings(signal);
      if (signal?.aborted) return;
      setSettings(data); setProvider(data.provider); setModel(data.model); setEnabled(data.enabled); setLimit(data.daily_request_limit); setConsent(false);
    } catch (e) { if (!signal?.aborted) setError(e instanceof Error ? e.message : 'Gagal memuat integrasi AI.'); }
    finally { if (!signal?.aborted) setLoading(false); }
  }, []);
  useEffect(() => { const controller = new AbortController(); void load(controller.signal); return () => controller.abort(); }, [load]);
  const selected = settings?.providers.find(p => p.id === provider);
  const changed = settings && (provider !== settings.provider || model !== settings.model || enabled !== settings.enabled || limit !== settings.daily_request_limit || key.length > 0);
  const save = async (remove = false) => {
    if (!settings) return;
    setBusy(true); setError(''); setMessage('');
    try {
      await saveAiSettings({ expected_revision: settings.revision, provider, model, enabled: remove ? false : enabled, daily_request_limit: limit,
        ...(remove ? { remove_api_key: true } : key ? { api_key: key.trim() } : {}) });
      setKey(''); setRemoving(false); await load();
      setMessage(text(remove ? 'Key dihapus dan integrasi dinonaktifkan.' : 'Pengaturan AI tersimpan.', remove ? 'Key removed and integration disabled.' : 'AI settings saved.'));
    } catch (e) { setError(e instanceof Error ? e.message : 'Gagal menyimpan pengaturan AI.'); }
    finally { setBusy(false); }
  };
  const test = async () => {
    if (!settings || !consent || changed) return;
    setBusy(true); setError(''); setMessage('');
    try {
      await testAiConnection(settings.revision); await load();
      setMessage(text('Koneksi dan output terstruktur berhasil diverifikasi.', 'Connection and structured output verified.'));
    } catch (e) { setError(e instanceof Error ? e.message : 'Uji koneksi gagal.'); }
    finally { setBusy(false); }
  };
  return <section className="ai-settings space-y-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-4 sm:p-6" aria-labelledby={`${id}-title`} data-testid="ai-settings">
    <header><h2 id={`${id}-title`} className="flex items-center gap-2 text-base font-semibold"><PlugZap size={18} aria-hidden="true" />{text('Integrasi AI', 'AI integration')}</h2>
      <p className="mt-2 text-sm text-[var(--ink-muted)]">{text('Hubungkan Gemini atau OpenAI untuk membantu menyusun paket hosting. Pengelolaan manual tetap tersedia.', 'Connect Gemini or OpenAI to help size hosting packages. Manual management remains available.')}</p></header>
    {loading ? <p role="status" className="flex items-center gap-2 text-sm"><Loader2 size={16} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />{text('Memuat pengaturan AI…', 'Loading AI settings…')}</p> : settings && <>
      <form className="space-y-4" onSubmit={event => { event.preventDefault(); void save(); }}>
        <label className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-1 size-4 shrink-0" checked={enabled} disabled={busy} onChange={e => setEnabled(e.target.checked)} /><span>{text('Aktifkan asisten AI untuk administrator', 'Enable the AI assistant for administrators')}</span></label>
        <div className="grid gap-4 sm:grid-cols-2">
          <div><label htmlFor={`${id}-provider`} className="text-sm">Provider</label><select id={`${id}-provider`} className={field} value={provider} disabled={busy} onChange={e => { const next = e.target.value as AiProvider; setProvider(next); setModel(settings.providers.find(p => p.id === next)?.models[0] ?? ''); setKey(''); setConsent(false); setMessage(''); }}>
            {settings.providers.map(p => <option key={p.id} value={p.id}>{p.id === 'openai' ? 'OpenAI' : 'Gemini'}</option>)}
          </select></div>
          <div><label htmlFor={`${id}-model`} className="text-sm">Model</label><select id={`${id}-model`} className={field} value={model} required disabled={busy} onChange={e => { setModel(e.target.value); setConsent(false); }}>{!selected?.models.includes(model) && <option value={model} disabled>{text('Model tersimpan tidak tersedia — pilih model', 'Saved model unavailable — choose a model')}</option>}{selected?.models.map(m => <option key={m}>{m}</option>)}</select></div>
        </div>
        <div><label htmlFor={`${id}-key`} className="text-sm">API key</label><input id={`${id}-key`} className={field} type="password" autoComplete="new-password" spellCheck={false} maxLength={512} value={key} disabled={busy} placeholder={selected?.has_key ? text('Key tersimpan · kosongkan untuk mempertahankan', 'Key saved · leave blank to keep it') : text('Masukkan API key provider', 'Enter the provider API key')} onChange={e => setKey(e.target.value)} />
          <p className="mt-2 text-xs text-[var(--ink-muted)]">{text('Key disimpan terenkripsi di server dan tidak ditampilkan kembali. Setiap provider menyimpan key terpisah.', 'Keys are encrypted on the server and never shown again. Each provider has its own key.')}</p></div>
        <div><label htmlFor={`${id}-limit`} className="text-sm">{text('Batas permintaan per hari', 'Daily request limit')}</label><input id={`${id}-limit`} className={field} type="number" min={1} max={1000} required value={limit} disabled={busy} onChange={e => setLimit(Number(e.target.value))} />
          <p className="mt-2 text-xs text-[var(--ink-muted)]">{text(`${settings.requests_today} percobaan hari ini; reset pukul 00.00 UTC. Uji koneksi dan percobaan gagal tetap dihitung.`, `${settings.requests_today} attempts today; resets at 00:00 UTC. Connection tests and failed attempts count.`)} {text(`Maksimum ${settings.max_output_tokens} token output per permintaan. Tagihan mengikuti provider.`, `Up to ${settings.max_output_tokens} output tokens per request. Provider billing applies.`)}</p></div>
        <div className="flex flex-wrap items-center gap-2"><button type="submit" className="button-primary min-h-11" disabled={busy || !changed || !selected?.models.includes(model)}>{busy ? text('Memproses…', 'Processing…') : text('Simpan pengaturan', 'Save settings')}</button>
          {selected?.has_key && <button type="button" className="button-secondary min-h-11" disabled={busy || Boolean(changed)} onClick={() => setRemoving(true)}>{text('Hapus key', 'Remove key')}</button>}</div>
      </form>
      <div className="border-t border-[var(--line)] pt-4 space-y-3"><label className="flex items-start gap-2 text-sm"><input type="checkbox" className="mt-1 size-4 shrink-0" checked={consent} disabled={busy || Boolean(changed)} onChange={e => setConsent(e.target.checked)} /><span>{text('Saya menyetujui satu permintaan uji ke provider terpilih. Uji dapat dikenai biaya; data server tidak dikirim.', 'I agree to one test request to the selected provider. It may incur charges; no server data is sent.')}</span></label>
        <button type="button" className="button-secondary min-h-11" disabled={busy || !consent || Boolean(changed) || !selected?.has_key} onClick={() => void test()}>{text('Uji koneksi', 'Test connection')}</button>
        {changed && <p className="text-xs text-[var(--ink-muted)]">{text('Simpan perubahan sebelum menguji koneksi.', 'Save changes before testing the connection.')}</p>}</div>
    </>}
    {message && <p role="status" className="text-sm">{message}</p>}
    {error && <div role="alert" className="space-y-2 text-sm text-[var(--danger)]"><p>{error}</p>{!settings && <button type="button" className="button-secondary min-h-11" onClick={() => void load()}>{text('Coba lagi', 'Retry')}</button>}</div>}
    <ConfirmDangerDialog isOpen={removing} onClose={() => setRemoving(false)} onConfirm={() => void save(true)} isLoading={busy} title={text('Hapus API key AI', 'Remove AI API key')} description={text('Key provider terpilih akan dihapus dan asisten dinonaktifkan. Paket hosting yang sudah ada tidak berubah.', 'The selected provider key will be removed and the assistant disabled. Existing hosting plans remain unchanged.')} confirmMatchText={provider === 'openai' ? 'OpenAI' : 'Gemini'} confirmButtonLabel={text('Hapus key', 'Remove key')} />
  </section>;
}
