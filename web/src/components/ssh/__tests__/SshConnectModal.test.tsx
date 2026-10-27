import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SshConnectModal, SshConnectionDetails } from '../SshConnectModal';
import * as client from '../../../api/client';
import { SshConnectionInfoResponse } from '../../../types/api';

describe('SshConnectModal Component', () => {
  const sampleFullInfo: SshConnectionInfoResponse = {
    host: 'server.example.com',
    port: 22,
    username: 'p_alice',
    user_id: 10,
    home_dir: '/home/p_alice',
    active_path: '/home/p_alice/sites/my-app/public',
    relative_path: 'sites/my-app/public',
    shell_policy: 'full',
    can_shell: true,
    can_sftp: true,
    has_ssh_keys: true,
    ssh_keys_count: 2,
    ssh_command:
      "ssh -t p_alice@server.example.com \"cd '/home/p_alice/sites/my-app/public' && exec \\$SHELL -l\"",
    sftp_command: "sftp p_alice@server.example.com:'/home/p_alice/sites/my-app/public'",
    sftp_cd_command: 'cd "/home/p_alice/sites/my-app/public"',
    sftp_url: 'sftp://p_alice@server.example.com/home/p_alice/sites/my-app/public',
    ssh_config_snippet:
      'Host my-app\n    HostName server.example.com\n    User p_alice\n    IdentityFile ~/.ssh/id_ed25519\n',
    site_domain: 'my-app.com',
    endpoint_state: 'configured',
    endpoint_id: 'ssh-fixture',
    endpoint_revision: 2,
    transport: 'direct',
    host_keys_status: 'local_configuration',
    host_keys: [{ key_type: 'ssh-ed25519', fingerprint_sha256: 'SHA256:fixture' }],
    sftp_path: '/home/p_alice/sites/my-app/public',
  };

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('renders the server endpoint and distinguishes local identity from reachability', () => {
    const html = renderToString(<SshConnectionDetails info={sampleFullInfo} />);
    for (const text of ['server.example.com', 'SHA256:fixture', 'belum membuktikan endpoint', 'Perintah SSH', 'Perintah SFTP', 'sintaks shell POSIX']) expect(html).toContain(text);
  });

  it('hides every generated guide when an endpoint is missing or uses Access', () => {
    for (const endpoint_state of ['missing', 'unsupported_transport'] as const) {
      const html = renderToString(<SshConnectionDetails info={{ ...sampleFullInfo, host: null, port: null, endpoint_state, transport: 'cloudflare_access' }} />);
      expect(html).not.toContain('Salin Perintah');
      expect(html).not.toContain('SHA256:fixture');
      expect(html).not.toContain('server.example.com');
    }
  });

  it('enforces sftp and none policies in the visible guides', () => {
    const sftp = renderToString(<SshConnectionDetails info={{ ...sampleFullInfo, shell_policy: 'sftp', can_shell: false, sftp_path: '/p_alice/sites/app' }} />);
    expect(sftp).not.toContain('Salin Perintah SSH');
    expect(sftp).toContain('Salin Perintah SFTP');
    expect(sftp).toContain('/p_alice/sites/app');
    const none = renderToString(<SshConnectionDetails info={{ ...sampleFullInfo, shell_policy: 'none', can_shell: false, can_sftp: false }} />);
    expect(none).not.toContain('Salin Perintah');
    expect(none).not.toContain('SHA256:fixture');
    expect(none).toContain('SSH dan SFTP dinonaktifkan');
  });

  it('marks manual previews as untrusted without borrowing the registered identity', () => {
    const html = renderToString(<SshConnectionDetails info={{ ...sampleFullInfo, endpoint_state: 'manual_preview', host_keys_status: 'untrusted_override', host_keys: [] }} />);
    expect(html).toContain('alamat ini belum dipercaya');
    expect(html).toContain('Fingerprint tidak tersedia untuk override manual');
    expect(html).not.toContain('SHA256:fixture');
  });

  it('does not render when isOpen is false', () => {
    const html = renderToString(
      <SshConnectModal isOpen={false} onClose={() => {}} />
    );
    expect(html).toBe('');
  });

  it('renders modal dialog with connection details when isOpen is true', () => {
    vi.spyOn(client, 'fetchSshConnectionInfo').mockResolvedValue(sampleFullInfo);

    const html = renderToString(
      <SshConnectModal
        isOpen={true}
        onClose={() => {}}
        path="/sites/my-app/public"
        siteDomain="my-app.com"
        tenantUsername="p_alice"
      />
    );

    expect(html).toContain('Koneksi SSH &amp; SFTP');
    expect(html).toContain('Memuat informasi koneksi...');
    expect(html).not.toContain('Parameter Koneksi Server');
  });

  it('does not fabricate SSH commands before connection data arrives', () => {
    vi.spyOn(client, 'fetchSshConnectionInfo').mockResolvedValue(sampleFullInfo);

    const html = renderToString(
      <SshConnectModal
        isOpen={true}
        onClose={() => {}}
        path="/sites/my-app/public"
        siteDomain="my-app.com"
        tenantUsername="p_alice"
      />
    );

    expect(html).not.toContain('ssh -t p_alice@');
    expect(html).not.toContain('Salin Perintah SSH (Buka di Folder Aktif)');
  });

  it('renders SFTP details and copy actions', () => {
    vi.spyOn(client, 'fetchSshConnectionInfo').mockResolvedValue(sampleFullInfo);

    const html = renderToString(
      <SshConnectModal
        isOpen={true}
        onClose={() => {}}
        path="/sites/my-app/public"
        siteDomain="my-app.com"
        tenantUsername="p_alice"
      />
    );

    expect(html).not.toContain('Salin Perintah SFTP CLI');
    expect(html).not.toContain('sftp p_alice@');
  });
});
