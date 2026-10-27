import { useId, useState } from 'react';
import type { EmailOverview } from '../../api/email';
import { RouteLink } from '../../lib/router';

type Certificate = EmailOverview['certificates'][number];

export function certificateCoversHostname(certificate: Certificate, hostname: string): boolean {
  const host = hostname.trim().toLowerCase();
  if (!host || !host.includes('.')) return false;
  return [certificate.hostname, ...certificate.san_domains].some(name => {
    const value = name.toLowerCase();
    return value === host || (value.startsWith('*.') && host.slice(host.indexOf('.') + 1) === value.slice(2));
  });
}

const input = 'w-full rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-sm text-[var(--ink)]';
const button = 'inline-flex items-center justify-center rounded-[var(--radius-md)] border border-[var(--line)] px-3 py-2 text-sm font-medium disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';

export function EmailServerSetup({ data, disabled, installing, onInstall }: {
  data: EmailOverview;
  disabled: boolean;
  installing: boolean;
  onInstall: (hostname: string, certificateId: number) => void;
}) {
  const id = useId();
  const [hostname, setHostname] = useState(data.server?.server?.hostname || '');
  const [certificateId, setCertificateId] = useState(String(data.server?.server?.certificate_id || ''));
  const host = hostname.trim().toLowerCase();
  const matching = data.certificates.filter(c => certificateCoversHostname(c, host));
  const selected = matching.some(c => String(c.id) === certificateId) ? certificateId : matching.length === 1 ? String(matching[0].id) : '';
  const sites = data.domains.filter(d => d.site_id && (!host || d.name === host));
  const uniqueSites = sites.filter((d, index) => sites.findIndex(other => other.site_id === d.site_id) === index);
  const hint = !data.certificates.length ? 'Belum ada sertifikat TLS yang dapat dipakai.' : host && !matching.length ? `Belum ada sertifikat yang mencakup ${host}.` : '';

  return <form className="space-y-4 border-t border-[var(--line)] pt-4" onSubmit={event => {
    event.preventDefault();
    if (!disabled && !installing && host && selected) onInstall(host, Number(selected));
  }}>
    <div className="space-y-1.5">
      <label htmlFor={`${id}-host`} className="block text-xs font-medium text-[var(--ink-muted)]">Hostname server email</label>
      <input id={`${id}-host`} className={input} value={hostname} placeholder="mail.example.com" required maxLength={253} disabled={disabled || installing} onChange={event => setHostname(event.target.value)} aria-describedby={`${id}-help`} />
    </div>
    <div className="space-y-1.5">
      <label htmlFor={`${id}-cert`} className="block text-xs font-medium text-[var(--ink-muted)]">Sertifikat TLS hostname email</label>
      <select id={`${id}-cert`} className={input} required value={selected} disabled={disabled || installing || !matching.length} onChange={event => setCertificateId(event.target.value)} aria-describedby={`${id}-help`}>
        <option value="">{!host ? 'Isi hostname email terlebih dahulu' : !matching.length ? 'Sertifikat untuk hostname ini belum tersedia' : 'Pilih sertifikat'}</option>
        {matching.map(c => <option key={c.id} value={c.id}>{c.hostname} (#{c.id})</option>)}
      </select>
    </div>
    <p id={`${id}-help`} className="text-xs leading-relaxed text-[var(--ink-muted)]">Gunakan sertifikat SSL project yang mencakup hostname email. HTTPS pada alamat panel saja tidak membuat sertifikat ini tersedia. Sertifikat kedaluwarsa, belum berlaku, dan Let's Encrypt staging tidak dapat dipilih.</p>
    {hint && <div className="space-y-3 border-y border-[var(--line)] py-4" role="status">
      <h3 className="text-sm font-semibold">{hint}</h3>
      <ol className="list-decimal space-y-2 pl-5 text-xs leading-relaxed text-[var(--ink-muted)]">
        <li>Arahkan A/AAAA hostname email ke IP publik server. Jika memakai Cloudflare, pilih DNS only untuk hostname email.</li>
        <li>Tambahkan hostname email sebagai domain pada situs milik tenant, atau buat situs statis khusus hostname tersebut.</li>
        <li>Buka tab SSL situs tersebut, lalu terbitkan Let's Encrypt produksi atau unggah sertifikat beserta private key yang sesuai. HTTP-01 memerlukan akses publik ke port 80.</li>
        <li>Kembali ke halaman ini dan tekan Muat ulang email. Pilih sertifikat, lalu pasang server email.</li>
      </ol>
      <div className="flex flex-wrap gap-3 text-xs">
        {uniqueSites.map(d => <RouteLink key={d.site_id} href={`/sites/${d.site_id}/ssl`} className="underline underline-offset-2">Buka SSL {d.name}</RouteLink>)}
        <RouteLink href="/sites" className="underline underline-offset-2">Buka daftar situs</RouteLink>
      </div>
    </div>}
    <button className={`${button} bg-[var(--primary)] text-[var(--primary-foreground)] hover:opacity-90`} disabled={disabled || installing || !host || !selected}>{installing ? 'Pemasangan sedang berjalan' : data.server?.configured ? 'Terapkan konfigurasi server' : 'Pasang server email'}</button>
  </form>;
}
