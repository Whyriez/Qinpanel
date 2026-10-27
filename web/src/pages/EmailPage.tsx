import React, { useState, useEffect, useCallback, useRef, useId } from 'react';
import { Mail, RefreshCw, Plus, Send, Server, Settings, ChevronLeft, Trash2, KeyRound, Globe, Copy, X } from 'lucide-react';
import { toast } from 'sonner';
import { useAuth } from '../context/AuthContext';
import { EmailServerSetup } from '../components/email/EmailServerSetup';
import { NotificationSettingsModal } from '../components/notifications/NotificationSettingsModal';
import { RuntimeJobProgress } from '../components/runtime/RuntimeJobProgress';
import { useRuntimeJobs } from '../components/runtime/useRuntimeJobs';
import { RouteLink } from '../lib/router';
import * as api from '../api/email';
import type { JobItem } from '../types/api';

const names = new Set(['email.install', 'email.mailbox', 'email.send', 'email.deliver']);
const input = 'w-full rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-sm text-[var(--ink)]';
const buttonBase = 'inline-flex items-center justify-center gap-2 rounded-[var(--radius-md)] border px-3 py-2 text-sm font-medium disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';
const button = `${buttonBase} border-[var(--line)] enabled:hover:bg-[var(--surface-sunken)]`;
const primary = `${buttonBase} border-[var(--primary)] bg-[var(--primary)] text-[var(--primary-foreground)] enabled:hover:bg-[var(--primary-hover)] enabled:hover:border-[var(--primary-hover)]`;
const card = 'rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 space-y-4 min-w-0';
const errorText = (e: unknown) => e instanceof Error ? e.message : 'Permintaan email gagal diproses.';
function Field({ label, children }: { label: string; children: React.ReactElement<{ id?: string }> }) {
  const id = useId(); return <div className="space-y-1.5"><label htmlFor={id} className="block text-xs font-medium text-[var(--ink-muted)]">{label}</label>{React.cloneElement(children, { id })}</div>;
}
const stateLabels = { pending: 'Sedang diproses', active: 'Aktif', disabled: 'Dinonaktifkan', failed: 'Perlu diperiksa' };

export function EmailPage() {
  const { user } = useAuth(); const admin = user?.role === 'admin';
  const [tab, setTab] = useState<'accounts' | 'notifications' | 'server'>('accounts');
  const [data, setData] = useState<api.EmailOverview | null>(null);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const submitting = useRef(false);
  const [selected, setSelected] = useState('');
  const [create, setCreate] = useState(false); const [localPart, setLocalPart] = useState(''); const [domain, setDomain] = useState('');
  const [pass, setPass] = useState(''); const [quota, setQuota] = useState(1024);
  const [credentials, setCredentials] = useState<{ address: string; password: string } | null>(null);
  const [smtpOpen, setSmtpOpen] = useState(false);
  const [template, setTemplate] = useState<api.EmailTemplate | null>(null);
  const [dns, setDns] = useState<api.EmailDns | null>(null);
  const [confirmDelete, setConfirmDelete] = useState('');
  const refresh = useCallback(async () => {
    try { const result = await api.fetchEmail(); setData(result); setError(''); return true; }
    catch (e) { setError(errorText(e)); return false; }
  }, []);
  const finished = useCallback(async (_job: JobItem) => refresh(), [refresh]);
  const jobs = useRuntimeJobs(finished, names);
  useEffect(() => { void refresh(); const timer = setInterval(() => { void refresh(); }, 10000); return () => clearInterval(timer); }, [refresh]);
  useEffect(() => { if (admin && tab === 'notifications') void api.fetchEmailTemplate().then(setTemplate).catch(e => setError(errorText(e))); }, [admin, tab]);
  const execute = async (fn: () => Promise<void>) => {
    if (submitting.current) return;
    submitting.current = true; setBusy(true); setError('');
    try { await fn(); } catch (e) { setError(errorText(e)); } finally { submitting.current = false; setBusy(false); }
  };
  const accept = async (result: api.EmailJob) => {
    jobs.track(result.job);
    if (result.address && result.password) setCredentials({ address: result.address, password: result.password });
    await refresh();
  };
  const account = data?.mailboxes.find(m => m.id === selected);
  const preview = template ? api.previewEmailTemplate(template) : null;
  const installJob = jobs.jobs.find(j => j.name === 'email.install');
  return <div className="space-y-6 text-[var(--ink)]" data-testid="email-page">
    <div className="flex items-start justify-between gap-4"><div><h1 className="text-xl font-bold">Email</h1><p className="mt-1 text-sm text-[var(--ink-muted)]">Kelola akun domain, baca pesan, dan atur email notifikasi panel.</p></div><button type="button" className={button} onClick={() => { void refresh(); }} aria-label="Muat ulang email"><RefreshCw size={16} /></button></div>
    <nav aria-label="Bagian email" className="flex flex-wrap gap-2 border-b border-[var(--line)] pb-3">
      <button type="button" aria-pressed={tab === 'accounts'} className={tab === 'accounts' ? primary : button} onClick={() => setTab('accounts')}><Mail size={16} />Akun dan kotak masuk</button>
      {admin && <><button type="button" aria-pressed={tab === 'notifications'} className={tab === 'notifications' ? primary : button} onClick={() => setTab('notifications')}><Settings size={16} />Notifikasi panel</button><button type="button" aria-pressed={tab === 'server'} className={tab === 'server' ? primary : button} onClick={() => setTab('server')}><Server size={16} />Server email</button></>}
    </nav>
    {error && <p role="alert" className="rounded-lg border border-[var(--danger)] p-3 text-sm text-[var(--danger)]">{error}</p>}
    {jobs.error && <p role="alert" className="text-sm text-[var(--danger)]">Progres belum dapat dimuat: {jobs.error}</p>}
    {jobs.jobs.length > 0 && <section className={card} aria-label="Tugas email"><h2 className="text-sm font-semibold">Proses email</h2>{jobs.jobs.slice(0, 4).map(job => <div key={job.id} className="space-y-2"><p className="text-xs">{job.name === 'email.install' ? 'Pemasangan server email' : job.name === 'email.mailbox' ? 'Perubahan akun email' : 'Pengiriman email'}</p><RuntimeJobProgress job={job} runtime="email" /></div>)}{jobs.jobs.length > 4 && <RouteLink href="/jobs" className="text-xs underline">Lihat {jobs.jobs.length - 4} tugas email lainnya</RouteLink>}</section>}
    {credentials && <section className={`${card} border-[var(--primary)]`} aria-label="Kredensial email baru"><div className="flex justify-between gap-3"><h2 className="font-semibold text-sm">Simpan password akun ini</h2><button type="button" className={button} onClick={() => setCredentials(null)} aria-label="Tutup kredensial"><X size={16} /></button></div><p className="text-xs text-[var(--ink-muted)]">Password hanya ditampilkan pada hasil perubahan ini. Tunggu tugas selesai sebelum masuk.</p><p className="break-all text-sm">{credentials.address}</p><code className="block break-all select-all text-sm">{credentials.password}</code><button type="button" className={button} onClick={() => { if (!navigator.clipboard) { toast.error('Pilih dan salin password secara manual. Fitur salin tidak tersedia pada browser ini.'); return; } void navigator.clipboard.writeText(credentials.password).then(() => toast.success('Password disalin')).catch(() => toast.error('Salin password secara manual')); }}><Copy size={14} />Salin password</button></section>}
    {!data && !error && <p role="status" className="text-sm text-[var(--ink-muted)]">Memuat email…</p>}
    {tab === 'accounts' && data && <>
      {(data.server_error || !data.server?.configured) && <section className={card}><h2 className="font-semibold text-sm">Server email belum siap</h2><p className="text-sm text-[var(--ink-muted)]">{data.server_error || 'Administrator perlu memasang layanan dan sertifikat TLS sebelum akun domain dapat dibuat.'}</p>{admin && <button type="button" className={button} onClick={() => setTab('server')}>Atur server email</button>}</section>}
      <div className="grid gap-5 xl:grid-cols-[minmax(280px,340px)_minmax(0,1fr)]">
        <section className={card} aria-label="Akun email"><div className="flex items-center justify-between gap-2"><h2 className="text-sm font-semibold">Akun email <span className="text-[var(--ink-muted)]">{data.mailboxes.length}</span></h2><button type="button" className={button} disabled={!data.server?.configured || busy || jobs.loading} onClick={() => setCreate(v => !v)}><Plus size={14} />Buat akun</button></div>
          {create && <form className="space-y-3 border-y border-[var(--line)] py-4" onSubmit={event => { event.preventDefault(); void execute(async () => { const result = await api.createMailbox({ domain_id: Number(domain), local_part: localPart, password: pass || undefined, quota_mb: quota }); await accept(result); setCreate(false); setLocalPart(''); setPass(''); if (result.id) setSelected(result.id); }); }}>
            <Field label="Nama akun"><input className={input} required maxLength={64} value={localPart} onChange={e => setLocalPart(e.target.value)} placeholder="admin" disabled={busy} /></Field>
            <Field label="Domain"><select className={input} required value={domain} onChange={e => setDomain(e.target.value)} disabled={busy}><option value="">Pilih domain</option>{data.domains.map(d => <option key={d.id} value={d.id}>{d.name}</option>)}</select></Field>
            {!data.domains.length && <p className="text-xs text-[var(--ink-muted)]">Tambahkan domain pada project milik tenant aktif terlebih dahulu.</p>}
            <Field label="Password (kosongkan untuk otomatis)"><input className={input} type="password" autoComplete="new-password" minLength={12} maxLength={128} value={pass} onChange={e => setPass(e.target.value)} disabled={busy} /></Field>
            <Field label="Kuota mailbox (MiB)"><input className={input} type="number" min={10} max={102400} required value={quota} onChange={e => setQuota(Number(e.target.value))} disabled={busy} /></Field>
            <p className="text-xs text-[var(--ink-muted)]">Kuota mailbox dikelola Dovecot. Kapasitas disk server tetap perlu dipantau.</p><button className={primary} disabled={busy || !domain}>Buat akun email</button>
          </form>}
          {!data.mailboxes.length && <p className="py-6 text-sm text-[var(--ink-muted)]">Belum ada akun email. Buat alamat menggunakan domain project Anda.</p>}
          <div className="space-y-2">{data.mailboxes.map(m => <button type="button" key={m.id} aria-pressed={selected === m.id} onClick={() => { setSelected(m.id); setDns(null); setConfirmDelete(''); }} className={`w-full rounded-lg border p-3 text-left ${selected === m.id ? 'border-[var(--primary)] bg-[var(--surface-sunken)]' : 'border-[var(--line)]'}`}><span className="block break-all text-sm font-medium">{m.address}</span><span className="mt-1 block text-xs text-[var(--ink-muted)]">{stateLabels[m.state]} · {m.quota_mb} MiB</span></button>)}</div>
        </section>
        <div className="min-w-0 space-y-5">{account ? <>
          <section className={card}><div><h2 className="break-all font-semibold">{account.address}</h2><p className="mt-1 text-xs text-[var(--ink-muted)]">{stateLabels[account.state]} · IMAP {data.server?.server?.hostname || '—'}:993 · SMTP submission :587 (STARTTLS)</p></div>
            {account.error && <p role="alert" className="text-sm text-[var(--danger)]">{account.error}</p>}
            {account.state === 'pending' && <p role="status" className="text-sm text-[var(--ink-muted)]">Perubahan sedang diproses. {account.job_id && <RouteLink href={`/jobs?job=${account.job_id}`} className="underline">Lihat log tugas</RouteLink>}</p>}
            <div className="flex flex-wrap gap-2">
              <button type="button" className={button} disabled={busy || account.state === 'pending'} onClick={() => { void execute(async () => { await accept(await api.changeMailbox(account.id, { action: 'reset_password' })); }); }}><KeyRound size={14} />Reset password</button>
              <button type="button" className={button} disabled={busy || account.state === 'pending'} onClick={() => { void execute(async () => { await accept(await api.changeMailbox(account.id, { action: 'set_enabled', enabled: !account.enabled })); }); }}>{account.enabled ? 'Nonaktifkan' : 'Aktifkan'}</button>
              {account.state === 'failed' && <button type="button" className={button} disabled={busy} onClick={() => { void execute(async () => { await accept(await api.changeMailbox(account.id, { action: 'retry' })); }); }}>Terapkan ulang</button>}
              <button type="button" className={button} disabled={busy || account.state !== 'active'} onClick={() => { void execute(async () => setDns(await api.fetchEmailDns(account.id))); }}><Globe size={14} />Panduan DNS</button>
              <button type="button" className={button} disabled={busy || account.state === 'pending'} onClick={() => setConfirmDelete(account.id)}><Trash2 size={14} />Hapus akun</button>
            </div>
            {confirmDelete === account.id && <div className="space-y-3 border-t border-[var(--line)] pt-4"><p className="text-sm">Hapus akses {account.address}? File pesan tetap disimpan di penyimpanan mail server untuk pemulihan manual.</p><div className="flex gap-2"><button type="button" className={button} disabled={busy} onClick={() => { void execute(async () => { await accept(await api.changeMailbox(account.id, { action: 'delete' })); setConfirmDelete(''); }); }}>Ya, hapus akun</button><button type="button" className={button} onClick={() => setConfirmDelete('')}>Batal</button></div></div>}
          </section>
          {dns && <section className={card} aria-label="Panduan DNS email"><div className="flex justify-between gap-3"><h3 className="text-sm font-semibold">Record DNS email</h3><button type="button" className={button} aria-label="Tutup panduan DNS" onClick={() => setDns(null)}><X size={14} /></button></div><p className="text-xs text-[var(--ink-muted)]">Buat A/AAAA untuk {dns.hostname} menuju {dns.ipv4 || dns.ipv6 || 'IP publik server'}. Pada Cloudflare, gunakan DNS only untuk hostname email. Periksa record existing sebelum menambahkan SPF.</p><div className="space-y-3">{dns.records.map((r, i) => <div key={i} className="border-t border-[var(--line)] pt-3 text-xs"><p className="font-semibold break-all">{r.type} · {r.name}{r.priority ? ` · prioritas ${r.priority}` : ''}</p><code className="mt-1 block break-all select-all">{r.content}</code></div>)}</div><p className="text-xs text-[var(--ink-muted)]">Atur PTR IP pada penyedia server. DNS dan tunnel HTTP tidak membuka port SMTP/IMAP. Panduan ini tidak mengubah DNS secara otomatis.</p>{admin && <RouteLink href="/settings" className="text-sm underline">Buka pengaturan DNS dan Cloudflare</RouteLink>}</section>}
          {account.state === 'active' && <Inbox key={account.id} account={account} onQueued={result => { jobs.track(result.job); }} revision={jobs.jobs.map(j => j.id + j.status).join(',')} />}
        </> : <section className={`${card} py-16 text-center`}><Mail className="mx-auto text-[var(--ink-muted)]" size={28} /><h2 className="text-sm font-semibold">Pilih akun email</h2><p className="text-sm text-[var(--ink-muted)]">Kotak masuk dan pengaturan akun akan tampil di sini.</p></section>}</div>
      </div>
    </>}
    {admin && tab === 'notifications' && <div className="grid gap-5 xl:grid-cols-2"><section className={card}><h2 className="font-semibold">Pengiriman notifikasi</h2><p className="text-sm text-[var(--ink-muted)]">Gunakan SMTP pilihan Anda untuk peringatan sertifikat, kuota, dan tugas gagal. Nama pengirim, alamat, serta penerima admin dapat disesuaikan.</p><button type="button" className={button} onClick={() => setSmtpOpen(true)}><Settings size={16} />Atur SMTP dan pengirim</button><p className="text-xs text-[var(--ink-muted)]">TLS/STARTTLS memeriksa sertifikat server. Tanpa enkripsi hanya tersedia untuk loopback. Pengiriman notifikasi tercatat pada Tugas.</p>
      {template && <form className="space-y-4 border-t border-[var(--line)] pt-4" onSubmit={e => { e.preventDefault(); void execute(async () => { await api.saveEmailTemplate(template); toast.success('Template email disimpan'); }); }}><h3 className="text-sm font-semibold">Template email custom</h3><Field label="Nama panel dalam email"><input className={input} maxLength={100} required value={template.panel_name} onChange={e => setTemplate({ ...template, panel_name: e.target.value })} /></Field><Field label="Subjek template"><input className={input} maxLength={300} required value={template.subject} onChange={e => setTemplate({ ...template, subject: e.target.value })} /></Field><Field label="Isi template (teks biasa)"><textarea className={input} rows={9} maxLength={16000} required value={template.body} onChange={e => setTemplate({ ...template, body: e.target.value })} /></Field><p className="text-xs leading-relaxed text-[var(--ink-muted)]">Placeholder: {'{panel_name}, {title}, {message}, {category}, {action_url}'}. Template disimpan sebagai teks biasa.</p><button className={primary} disabled={busy}>Simpan template</button></form>}
    </section><section className={card}><h2 className="font-semibold">Pratinjau notifikasi</h2>{preview ? <><p className="text-sm font-medium break-words">{preview.subject}</p><pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-[var(--ink-muted)]">{preview.body}</pre></> : <p className="text-sm text-[var(--ink-muted)]">Memuat template…</p>}</section></div>}
    {admin && tab === 'server' && data && <div className="grid gap-5 xl:grid-cols-2"><section className={card}><h2 className="font-semibold">Layanan email domain</h2><p className="text-sm text-[var(--ink-muted)]">Postfix menerima dan mengantrekan email, Dovecot menyediakan kotak masuk IMAP, dan OpenDKIM menandatangani pesan keluar.</p>{data.server_error && <p role="alert" className="text-sm text-[var(--danger)]">{data.server_error}</p>}<dl className="divide-y divide-[var(--line)]">{data.server?.services.map(s => <div key={s.name} className="flex justify-between py-3 text-sm"><dt>{s.name}</dt><dd>{s.running ? 'Berjalan' : 'Tidak berjalan'}</dd></div>)}</dl><p className="text-sm">Konfigurasi: {data.server?.configured ? 'Siap' : 'Belum siap'}</p>
      <EmailServerSetup data={data} disabled={busy || jobs.loading} installing={!!installJob} onInstall={(hostname, certificateId) => { void execute(async () => { await accept(await api.installEmail(hostname, certificateId)); }); }} />
    </section><section className={card}><h2 className="font-semibold">Sebelum menerima email publik</h2><ul className="list-disc pl-5 space-y-3 text-sm text-[var(--ink-muted)]"><li>Gunakan IP publik yang menerima port 25, 587, dan 993. Periksa batas SMTP keluar pada penyedia server.</li><li>Arahkan hostname mail melalui A/AAAA dengan DNS only, lalu tambahkan MX, SPF, DKIM, dan DMARC dari panduan setiap akun.</li><li>Atur reverse DNS (PTR) ke hostname mail melalui penyedia IP server.</li><li>Gunakan username berupa alamat email lengkap. IMAP memakai TLS pada port 993, SMTP memakai STARTTLS pada port 587.</li><li>Instalasi hanya mengelola konfigurasi baru atau konfigurasi milik Qinpanel. Server mail existing perlu migrasi terpisah.</li></ul><p className="border-t border-[var(--line)] pt-4 text-xs text-[var(--ink-muted)]">Status layanan tidak membuktikan email diterima server tujuan. Periksa antrean Postfix, DNS, sertifikat, dan reputasi IP saat menguji pengiriman publik.</p></section></div>}
    <NotificationSettingsModal isOpen={smtpOpen} onClose={() => setSmtpOpen(false)} />
  </div>;
}

function Inbox({ account, onQueued, revision }: { account: api.Mailbox; onQueued: (r: api.EmailJob) => void; revision: string }) {
  const [folder, setFolder] = useState<api.EmailFolder>('inbox'); const [list, setList] = useState<api.EmailMessages | null>(null);
  const [message, setMessage] = useState<(api.EmailItem & { uid_validity: number }) | null>(null); const [cursor, setCursor] = useState<number>();
  const [error, setError] = useState(''); const [loading, setLoading] = useState(false); const [busy, setBusy] = useState(false); const submitting = useRef(false); const generation = useRef(0);
  const [compose, setCompose] = useState(false); const [draft, setDraft] = useState(() => ({ request_id: api.createEmailRequestId(), to: '', subject: '', body: '' }));
  const load = useCallback(async () => {
    const current = ++generation.current; setLoading(true);
    try { const result = await api.fetchEmailMessages(account.id, folder, cursor); if (current === generation.current) { setList(result); setError(''); } }
    catch (e) { if (current === generation.current) setError(errorText(e)); } finally { if (current === generation.current) setLoading(false); }
  }, [account.id, folder, cursor]);
  useEffect(() => { setMessage(null); setList(null); void load(); return () => { generation.current++; }; }, [load]);
  useEffect(() => { void load(); const timer = setInterval(() => { void load(); }, 15000); return () => clearInterval(timer); }, [load, revision]);
  const action = async (fn: () => Promise<void>) => { if (submitting.current) return; submitting.current = true; setBusy(true); setError(''); try { await fn(); } catch (e) { setError(errorText(e)); } finally { submitting.current = false; setBusy(false); } };
  return <section className={card} aria-label="Kotak masuk email"><div className="flex flex-wrap justify-between gap-3"><div className="flex gap-2">{([['inbox', 'Kotak masuk'], ['sent', 'Terkirim'], ['trash', 'Sampah']] as const).map(([key, label]) => <button type="button" key={key} className={folder === key ? primary : button} aria-pressed={folder === key} disabled={busy} onClick={() => { setFolder(key); setCursor(undefined); }}>{label}</button>)}</div><div className="flex gap-2"><button type="button" className={button} disabled={loading || busy} onClick={() => { setMessage(null); void load(); }} aria-label="Muat ulang kotak masuk"><RefreshCw size={15} /></button><button type="button" className={primary} disabled={busy} onClick={() => setCompose(v => !v)}><Plus size={14} />Tulis email</button></div></div>
    {error && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
    {compose && <form className="space-y-3 border-y border-[var(--line)] py-4" onSubmit={e => { e.preventDefault(); void action(async () => { const result = await api.sendEmail(account.id, draft); onQueued(result); setCompose(false); setDraft({ request_id: api.createEmailRequestId(), to: '', subject: '', body: '' }); toast.success('Pesan masuk antrean pengiriman. Lihat status di Tugas.'); }); }}><h3 className="text-sm font-semibold">Pesan baru dari {account.address}</h3><Field label="Kepada"><input className={input} type="email" required value={draft.to} disabled={busy} onChange={e => setDraft({ ...draft, request_id: api.createEmailRequestId(), to: e.target.value })} /></Field><Field label="Subjek"><input className={input} maxLength={512} required value={draft.subject} disabled={busy} onChange={e => setDraft({ ...draft, request_id: api.createEmailRequestId(), subject: e.target.value })} /></Field><Field label="Isi pesan"><textarea className={input} rows={7} maxLength={1000000} required value={draft.body} disabled={busy} onChange={e => setDraft({ ...draft, request_id: api.createEmailRequestId(), body: e.target.value })} /></Field><p className="text-xs text-[var(--ink-muted)]">Pesan dikirim sebagai teks biasa. Lampiran dapat dikirim melalui aplikasi IMAP/SMTP.</p><div className="flex gap-2"><button className={primary} disabled={busy}><Send size={14} />{busy ? 'Memasukkan ke antrean…' : 'Kirim email'}</button><button type="button" className={button} disabled={busy} onClick={() => setCompose(false)}>Tutup draf</button></div></form>}
    {message ? <article className="space-y-4"><button type="button" className={button} onClick={() => setMessage(null)}><ChevronLeft size={14} />Daftar pesan</button><h3 className="break-words font-semibold">{message['hdr.subject'] || '(Tanpa subjek)'}</h3><p className="text-xs break-words text-[var(--ink-muted)]">Dari: {message['hdr.from']}<br />Kepada: {message['hdr.to']}</p><pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed border-t border-[var(--line)] pt-4">{message['text.utf8'] || '(Pesan kosong)'}</pre>{folder !== 'trash' && <button type="button" className={button} disabled={busy || !list} onClick={() => { void action(async () => { await api.emailMessage(account.id, 'trash', folder, Number(message.uid), message.uid_validity); setMessage(null); await load(); }); }}><Trash2 size={14} />Pindahkan ke Sampah</button>}<p className="text-xs text-[var(--ink-muted)]">Tampilan teks aman. HTML, gambar eksternal, dan lampiran tidak dijalankan.</p></article> : <>
      <div className="flex justify-between gap-2 text-xs text-[var(--ink-muted)]"><span>{list ? `${list.total} pesan · ${list.unseen} belum dibaca` : loading ? 'Memuat pesan…' : 'Daftar pesan belum tersedia'}</span>{loading && list && <span role="status">Memperbarui…</span>}</div>
      {list?.items.length === 0 && <p className="py-10 text-center text-sm text-[var(--ink-muted)]">Tidak ada pesan pada rentang ini.</p>}
      <div className="divide-y divide-[var(--line)]">{list?.items.map(m => <button type="button" key={m.uid} className="w-full py-3 text-left hover:bg-[var(--surface-sunken)] disabled:opacity-50" disabled={busy} onClick={() => { void action(async () => { const current = generation.current; const detail = await api.emailMessage(account.id, 'read', folder, Number(m.uid), list.uid_validity); if (current === generation.current) setMessage({ ...detail, uid_validity: list.uid_validity }); }); }}><span className="block break-words text-sm font-medium">{m['hdr.subject'] || '(Tanpa subjek)'}</span><span className="mt-1 block break-words text-xs text-[var(--ink-muted)]">{m['hdr.from']}</span></button>)}</div>
      <div className="flex gap-2">{cursor && <button type="button" className={button} disabled={loading || busy} onClick={() => setCursor(undefined)}>Pesan terbaru</button>}{list?.before && <button type="button" className={button} disabled={loading || busy} onClick={() => setCursor(list.before!)}>Pesan lebih lama</button>}</div>
    </>}
  </section>;
}
