import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteBackupsTab } from '../SiteBackupsTab';
import '../../../i18n';

describe('SiteBackupsTab Component', () => {
  it('renders initial backup tab structure with summary cards, search, and action buttons', () => {
    const html = renderToString(
      <SiteBackupsTab siteId={1} siteDomain="example.com" />
    );

    // Summary cards
    expect(html).toContain('Total Penggunaan Ruang Cadangan');
    expect(html).toContain('Pencadangan Otomatis');
    expect(html).toContain('Tujuan Arsip');

    // Action buttons
    expect(html).toContain('Buat Cadangan');
    expect(html).toContain('Jadwal &amp; Retensi');

    // Search input
    expect(html).toContain('Cari berkas atau label cadangan...');
  });

  it('renders loading state cleanly on initial render', () => {
    const html = renderToString(<SiteBackupsTab siteId={1} />);
    expect(html).toContain('Memuat daftar arsip cadangan...');
  });
});
