import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { RuntimePage } from '../RuntimePage';
import { PhpExtensionsModal } from '../../components/runtime/PhpExtensionsModal';
import { NodeInstallSettings, defaultNodeInstallOptions } from '../../components/runtime/NodeInstallSettings';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('RuntimePage Component', () => {
  it('renders PHP runtime manager header, repository status, and security notice', () => {
    const html = renderToString(
      <AuthProvider>
        <RuntimePage />
      </AuthProvider>
    );

    expect(html).toContain('Manajer Runtime PHP');
    expect(html).toContain('Status repositori PHP');
    expect(html).toContain('Isolasi FPM Multi-Tenant &amp; Kebijakan Masking OS');
    expect(html).toContain('Konfigurasi repositori PHP');
  });

  it('keeps language tabs beside Settings and moves scheduling out of PHP', () => {
    const html = renderToString(
      <AuthProvider>
        <RuntimePage />
      </AuthProvider>
    );

    const navigation = html.match(/<nav aria-label="Navigasi runtime"[^>]*>(.*?)<\/nav>/)?.[1] ?? '';
    const labels = [...navigation.matchAll(/<span>([^<]+)<\/span>/g)].map(match => match[1]);
    expect(labels).toEqual(['PHP', 'Node.js', 'Python', 'Pengaturan', 'Monitoring Proses', 'Server Basis Data']);
    expect(navigation).toContain('aria-pressed="true"');
    expect(html).not.toContain('Jadwal katalog runtime');
  });

  it('renders PhpExtensionsModal with version title and category filter buttons', () => {
    const html = renderToString(
      <PhpExtensionsModal
        version="8.3"
        isOpen={true}
        onClose={() => {}}
      />
    );

    expect(html).toContain('Ekstensi PHP 8.3');
    expect(html).toContain('Semua Kategori');
    expect(html).toContain('Total:');
    expect(html).toContain('Terpasang:');
    expect(html).toContain('Aktif:');
  });

  it('retains optional Node download settings inline with Corepack enabled by default', () => {
    const html = renderToString(
      <NodeInstallSettings value={defaultNodeInstallOptions} disabled={false} onChange={() => {}} />
    );
    expect(html).toContain('checked=""');
    expect(html).toContain('Mirror unduhan (opsional)');
    expect(html).toContain('Arsip offline di server');
    expect(html).toContain('Checksum SHA256');
    expect(html).not.toContain('role="dialog"');
  });
});
