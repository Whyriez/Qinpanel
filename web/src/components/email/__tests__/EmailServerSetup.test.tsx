import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import type { EmailOverview } from '../../../api/email';
import { certificateCoversHostname, EmailServerSetup } from '../EmailServerSetup';

const empty: EmailOverview = { mailboxes: [], domains: [{ id: 1, name: 'mail.example.com', site_id: 7 }], server: null, server_error: null, certificates: [] };
const render = (data: EmailOverview) => renderToString(<EmailServerSetup data={data} disabled={false} installing={false} onInstall={() => undefined} />);

describe('email TLS setup', () => {
  it('explains the missing certificate and links directly to project SSL', () => {
    const html = render(empty);
    expect(html).toContain('Belum ada sertifikat TLS yang dapat dipakai');
    expect(html).toContain('href="/sites/7/ssl"');
    expect(html).toContain('DNS only');
    expect(html).toContain('port 80');
    expect(html).toContain('disabled="">Pasang server email');
    expect(html).not.toContain('Sertifikat TLS panel');
  });

  it('matches SANs and one wildcard label, without accepting unrelated names', () => {
    const cert = { id: 1, hostname: 'example.com', san_domains: ['*.example.com', 'mail.other.test'] };
    expect(certificateCoversHostname(cert, ' MAIL.EXAMPLE.COM ')).toBe(true);
    expect(certificateCoversHostname(cert, 'mail.other.test')).toBe(true);
    for (const host of ['', 'localhost', 'deep.mail.example.com', 'example.com.attacker.test', 'other.test']) {
      expect(certificateCoversHostname(cert, host), host).toBe(false);
    }
  });

  it('preloads the configured server and offers only matching certificates', () => {
    const html = render({ ...empty, server: { configured: true, server: { hostname: 'mail.example.com', certificate_id: 5 }, services: [] }, certificates: [{ id: 5, hostname: 'example.com', san_domains: ['mail.example.com'] }, { id: 6, hostname: 'other.test', san_domains: [] }] });
    expect(html).toContain('value="mail.example.com"');
    expect(html).toContain('value="5" selected=""');
    expect(html).not.toContain('value="6"');
    expect(html).toContain('Terapkan konfigurasi server');
    expect(html).not.toContain('Belum ada sertifikat');
  });
});
