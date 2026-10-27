import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteDatabaseTab } from '../SiteDatabaseTab';
import '../../../i18n';

describe('SiteDatabaseTab Component', () => {
  it('renders initial database tab structure with quota bar, search, and action button', () => {
    const html = renderToString(
      <SiteDatabaseTab siteId={1} siteDomain="example.com" />
    );

    // Title & Subtitle
    expect(html).toContain('Manajemen Basis Data');
    expect(html).toContain('Basis data terisolasi untuk situs example.com');

    // Action button
    expect(html).toContain('Buat Basis Data');

    // Quota indicator
    expect(html).toContain('Penggunaan Kuota Basis Data');

    // Engine filter buttons
    expect(html).toContain('MariaDB');
    expect(html).toContain('PostgreSQL');
    expect(html).toContain('Semua');

    // Search input
    expect(html).toContain('Cari nama basis data...');
  });

  it('renders loading state cleanly on initial render', () => {
    const html = renderToString(<SiteDatabaseTab siteId={1} />);
    expect(html).toContain('Memuat daftar basis data...');
    expect(html).toContain('Memuat kuota...');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[\s\S]*?Buat Basis Data/);
    expect(html).not.toContain('aria-valuemax="5"');
  });
});
