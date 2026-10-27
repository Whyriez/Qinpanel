import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteRulesTab, DEFAULT_RULES } from '../SiteRulesTab';
import { Site } from '../../../types/api';
import '../../../i18n';

const mockSite: Site = {
  id: 1,
  user_id: 1,
  primary_domain: 'example.com',
  kind: 'php',
  version: '8.2',
  status: 'active',
  docroot: '/var/www/vhosts/user1/example.com',
  spa_fallback: false,
  force_https: true,
  domains: [],
  created_at: 1711792800,
  updated_at: 1711792800,
};

describe('SiteRulesTab Component', () => {
  it('renders initial loading state cleanly when fetching rules', () => {
    const html = renderToString(
      <SiteRulesTab site={mockSite} userRole="admin" />
    );
    expect(html).toContain('Memuat aturan situs...');
  });

  it('renders site rules navigation tabs for all rule sections when loaded', () => {
    const html = renderToString(
      <SiteRulesTab site={mockSite} userRole="admin" initialRules={DEFAULT_RULES} />
    );

    // Header & title
    expect(html).toContain('Aturan Situs &amp; Keamanan Web');
    expect(html).toContain('Simpan Perubahan Aturan');

    // Section sub-tabs
    expect(html).toContain('Redirect');
    expect(html).toContain('Header');
    expect(html).toContain('CORS');
    expect(html).toContain('Hotlink');
    expect(html).toContain('Basic Auth');
    expect(html).toContain('Akses IP');
    expect(html).toContain('Maintenance');
    expect(html).toContain('Halaman Error');
    expect(html).toContain('Snippet Nginx');
  });

  it('renders default redirect section content cleanly', () => {
    const html = renderToString(
      <SiteRulesTab site={mockSite} userRole="tenant" initialRules={DEFAULT_RULES} />
    );
    expect(html).toContain('Redirect Terstruktur (301 / 302)');
    expect(html).toContain('Tambah Redirect');
    expect(html).toContain('Belum ada aturan redirect yang dibuat');
  });

  it('renders redirect source and target from the Rust API contract', () => {
    const html = renderToString(<SiteRulesTab site={mockSite} initialRules={{
      ...DEFAULT_RULES,
      redirects: [{ id: 'saved-id', source: '/old', target: '/new', status_code: 302, match_type: 'exact', enabled: false }],
    }} />);
    expect(html).toContain('/old');
    expect(html).toContain('/new');
    expect(html).toContain('302');
  });
});
