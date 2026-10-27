import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteDeployTab } from '../SiteDeployTab';
import { SiteGitConfig } from '../../../types/api';
import '../../../i18n';

const mockConfig: SiteGitConfig = {
  id: 1,
  site_id: 1,
  repo_url: 'git@github.com:myorg/webapp.git',
  branch: 'production',
  build_cmd: 'npm run build',
  post_deploy_cmd: 'npm run migrate',
  docroot_subpath: 'dist',
  auto_deploy: true,
  keep_releases: 5,
  deploy_key_public: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExamplePublicKey webapp@nusa',
  webhook_url: '/api/v1/webhooks/deploy/1',
  webhook_secret: 'abcdef0123456789abcdef0123456789',
  has_webhook_secret: true,
  last_deployed_at: 1711792800,
  created_at: 1711792800,
  updated_at: 1711792800,
};

describe('SiteDeployTab Component', () => {
  it('renders initial Git deploy structure with header and primary cards', () => {
    const html = renderToString(<SiteDeployTab siteId={1} />);

    // Header & Subtitle
    expect(html).toContain('Git Deployment &amp; Webhook');
    expect(html).toContain('Deploy otomatis dan zero-downtime atomic symlink swap');

    // Action buttons
    expect(html).toContain('Deploy Sekarang');

    // Cards
    expect(html).toContain('Konfigurasi Repositori Git');
    expect(html).toContain('Kunci Deploy Ed25519');
    expect(html).toContain('Integrasi Webhook');
    expect(html).toContain('Riwayat Deployment');
  });

  it('renders with populated configuration', () => {
    const html = renderToString(
      <SiteDeployTab siteId={1} initialConfig={mockConfig} />
    );

    // Repo url & branch
    expect(html).toContain('git@github.com:myorg/webapp.git');
    expect(html).toContain('production');
    expect(html).toContain('npm run build');

    // Deploy key
    expect(html).toContain('ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExamplePublicKey webapp@nusa');

    // Webhook instructions
    expect(html).toContain('URL Webhook');
    expect(html).toContain('/api/v1/webhooks/deploy/1');
    expect(html).toContain('Webhook Secret (HMAC-SHA256)');
  });
});
