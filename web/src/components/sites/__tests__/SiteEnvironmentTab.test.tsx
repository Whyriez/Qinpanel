import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteEnvironmentTab } from '../SiteEnvironmentTab';
import '../../../i18n';

describe('SiteEnvironmentTab Component', () => {
  it('renders environment tab with loading state initially', () => {
    const html = renderToString(<SiteEnvironmentTab siteId={1} />);
    expect(html).toContain('Variabel Lingkungan (Environment Variables)');
    expect(html).toContain('XChaCha20-Poly1305');
    expect(html).toContain('Tambah Variabel');
    expect(html).toContain('Import .env');
    expect(html).toContain('Tampilkan Semua');
  });

  it('contains expected structure and accessibility labels', () => {
    const html = renderToString(<SiteEnvironmentTab siteId={1} />);
    expect(html).toContain('Cari variabel...');
    expect(html).toContain('variabel terenkripsi');
  });
});
