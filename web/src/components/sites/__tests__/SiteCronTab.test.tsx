import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteCronTab } from '../SiteCronTab';
import '../../../i18n';

describe('SiteCronTab Component', () => {
  it('renders initial cron tab structure with header, quota badge, and search', () => {
    const html = renderToString(<SiteCronTab siteId={1} />);

    // Header & Subtitle
    expect(html).toContain('Tugas Terjadwal (Cron Job)');
    expect(html).toContain('Berjalan terisolasi sebagai tenant di systemd slice');

    // Quota Badge & Add button
    expect(html).toContain('Batas Quota:');
    expect(html).toContain('Tambah Cron Job');

    // Search and Refresh controls
    expect(html).toContain('Cari cron job...');
    expect(html).toContain('Segarkan');
  });

  it('renders empty state or loading container cleanly', () => {
    const html = renderToString(<SiteCronTab siteId={1} />);
    // In server renderToString before useEffect runs:
    // loading state or empty state is present
    expect(html).toMatch(/Memuat\.\.\.|Belum Ada Cron Job/);
  });
});
