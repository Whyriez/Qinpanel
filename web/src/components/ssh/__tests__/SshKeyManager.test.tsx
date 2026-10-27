import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SshKeyManager } from '../SshKeyManager';
import { SshKeyItem } from '../../../types/api';
import * as client from '../../../api/client';

describe('SshKeyManager Component', () => {
  const sampleKeys: SshKeyItem[] = [
    {
      id: 1,
      user_id: 10,
      name: 'Laptop Kerja Ed25519',
      key_type: 'ssh-ed25519',
      public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... dev@work',
      fingerprint_sha256: 'SHA256:4tL9Xh8XyK7Bv4Z5qT1mP8W2aL0cE9uG5fK2jX7bV4m',
      fingerprint_md5: 'md5:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00',
      created_at: 1700000000,
    },
    {
      id: 2,
      user_id: 10,
      name: 'CI Pipeline RSA',
      key_type: 'ssh-rsa',
      public_key: 'ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC... ci@runner',
      fingerprint_sha256: 'SHA256:8kM2Nj8XyK7Bv4Z5qT1mP8W2aL0cE9uG5fK2jX7bV9p',
      fingerprint_md5: 'md5:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99',
      created_at: 1700100000,
    },
  ];

  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('renders header, description, and keygen helper banner', () => {
    vi.spyOn(client, 'fetchSshKeys').mockResolvedValue(sampleKeys);

    const html = renderToString(<SshKeyManager />);

    expect(html).toContain('Kunci SSH Publik');
    expect(html).toContain('Cara membuat pasangan kunci SSH di komputer lokal Anda:');
    expect(html).toContain('ssh-keygen -t ed25519');
    expect(html).toContain('Tambah Kunci SSH');
  });

  it('hides add button when in readOnly mode', () => {
    vi.spyOn(client, 'fetchSshKeys').mockResolvedValue(sampleKeys);

    const html = renderToString(<SshKeyManager readOnly={true} />);

    expect(html).toContain('Kunci SSH Publik');
    expect(html).not.toContain('Tambah Kunci SSH');
  });

  it('renders loading feedback and explains account access', () => {
    // Note: In SSR / initial renderToString, useState keys defaults to [] and isLoading is true.
    // Let's verify initial loading skeleton or empty rendering.
    const html = renderToString(<SshKeyManager />);

    // Renders the loading or container frame
    expect(html).toContain('Memuat kunci SSH terdaftar...');
    expect(html).toContain('SSH atau SFTP sesuai izin akses akun Anda');
  });
});
