import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { CommandPalette } from '../CommandPalette';
import '../../../i18n';

const auth = vi.hoisted(() => ({ role: 'user' }));
vi.mock('../../../context/AuthContext', () => ({ useAuth: () => ({ user: { id: 42, role: auth.role }, logout: vi.fn() }) }));

describe('CommandPalette Component', () => {
  it('renders command palette items when open', () => {
    const onNavigate = vi.fn();
    const onThemeChange = vi.fn();

    const html = renderToString(
        <CommandPalette
          isOpen={true}
          onNavigate={onNavigate}
          theme="light"
          onThemeChange={onThemeChange}
        />
    );

    expect(html).toContain('Pencarian cepat');
    expect(html).toContain('Navigasi');
    expect(html).toContain('Preferensi');
    expect(html).toContain('Tema: Terang');
    expect(html).toContain('Tema: Gelap');
    expect(html).toContain('Tutup pencarian');
    expect(html).toContain('Pindah pilihan');
  });

  it('renders nothing when closed', () => {
    const onNavigate = vi.fn();
    const onThemeChange = vi.fn();

    const html = renderToString(
        <CommandPalette
          isOpen={false}
          onNavigate={onNavigate}
          theme="light"
          onThemeChange={onThemeChange}
        />
    );

    expect(html).toBe('');
  });

  it('shows tenant analytics and descriptions while excluding admin commands', () => {
    auth.role = 'user';
    const html = renderToString(<CommandPalette isOpen onNavigate={vi.fn()} theme="light" onThemeChange={vi.fn()} activeNav="analytics" />);
    expect(html).toContain('Analitik');
    expect(html).toContain('Lalu lintas dan performa situs');
    expect(html).toContain('Halaman ini');
    for (const description of ['Akun dan ruang hosting', 'Batas sumber daya dan akses', 'Versi PHP dan Node.js', 'Catatan aktivitas panel', 'Konfigurasi panel dan keamanan']) expect(html).not.toContain(description);
  });

  it('provides the full administration group for administrators', () => {
    auth.role = 'admin';
    const html = renderToString(<CommandPalette isOpen onNavigate={vi.fn()} theme="dark" onThemeChange={vi.fn()} />);
    expect(html).toContain('Administrasi');
    for (const description of ['Akun dan ruang hosting', 'Batas sumber daya dan akses', 'Versi PHP dan Node.js', 'Catatan aktivitas panel', 'Konfigurasi panel dan keamanan']) expect(html).toContain(description);
    expect(html).toContain('command-active');
    auth.role = 'user';
  });
});
