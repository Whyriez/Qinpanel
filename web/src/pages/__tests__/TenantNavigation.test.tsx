import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Shell } from '../../components/layout/Shell';
import { JobsPage } from '../JobsPage';
import { ConnectivityPage } from '../ConnectivityPage';
import { ConnectionAddress, ProjectConnectionInfo } from '../../components/sites/ProjectConnectionInfo';
import type { Endpoint } from '../../api/connectivity';
import type { Site } from '../../types/api';
import i18n from '../../i18n';

const auth = vi.hoisted(() => ({ role: 'user' }));
vi.mock('../../context/AuthContext', () => ({ useAuth: () => ({ user: { id: 42, username: 'alice', role: auth.role }, logout: vi.fn() }) }));
const endpoint: Endpoint = { id: 'registered', revision: 2, service: 'ssh', scope: { kind: 'project', site_id: 7 }, host: '2001:db8::1', port: 2222, transport: 'cloudflare_access', tls_mode: 'ssh', certificate_id: null, enabled: true, primary: true, client_requirement: 'cloudflared', created_at: 1, updated_at: 2 };

beforeEach(async () => { auth.role = 'user'; await i18n.changeLanguage('id'); });
describe('Tenant activity and project connections', () => {
  it('labels tenant navigation and the breadcrumb as activity and removes global connectivity', () => {
    const html = renderToStaticMarkup(<Shell theme="light" onThemeChange={vi.fn()} activeNav="jobs"><p>Content</p></Shell>);
    expect(html).toContain('href="/jobs"');
    expect(html).toContain('aria-label="Aktivitas"');
    expect(html).toContain('<strong>Aktivitas</strong>');
    expect(html).not.toContain('href="/connectivity"');
    expect(html).not.toContain('aria-label="Tugas"');
  });
  it('explains useful tenant operations in the activity page', () => {
    const html = renderToStaticMarkup(<JobsPage />);
    expect(html).toContain('Aktivitas');
    expect(html).toContain('backup, impor dan ekspor database');
    expect(html).toContain('Belum ada aktivitas');
    expect(html).not.toContain('Antrean Tugas Sistem');
  });
  it('keeps administrator tasks and connectivity management', () => {
    auth.role = 'admin';
    expect(renderToStaticMarkup(<JobsPage />)).toContain('Antrean Tugas Sistem');
    expect(renderToStaticMarkup(<Shell theme="light" onThemeChange={vi.fn()} activeNav="jobs">Content</Shell>)).toContain('aria-label="Tugas"');
    const html = renderToStaticMarkup(<ConnectivityPage />);
    expect(html).toContain('Konektivitas');
    expect(html).not.toContain('Pilih project');
  });
  it('keeps old tenant connectivity links useful through project selection', () => {
    const html = renderToStaticMarkup(<ConnectivityPage />);
    expect(html).toContain('Informasi koneksi');
    expect(html).toContain('Pilih project');
    expect(html).not.toContain('Tambah alamat');
  });
  it('links guides to the current project without inventing endpoint addresses', () => {
    const site = { id: 7, primary_domain: 'project.example.test' } as Site;
    const html = renderToStaticMarkup(<ProjectConnectionInfo site={site} onOpenSsh={vi.fn()} />);
    expect(html).toContain('Panduan SSH / SFTP');
    expect(html).toContain('/sites/project.example.test/database');
    expect(html).toContain('/sites/project.example.test/storage');
    expect(html).not.toContain('project.example.test:22');
    expect(html).not.toContain('Belum diatur');
  });
  it('shows registered IPv6, port and client requirements without mutation or reachability claims', () => {
    const html = renderToStaticMarkup(<ConnectionAddress endpoint={endpoint} />);
    expect(html).toContain('[2001:db8::1]:2222');
    expect(html).toContain('Memerlukan cloudflared');
    expect(html).toContain('Salin alamat');
    expect(html).not.toMatch(/>Ubah|>Hapus|>Periksa|Terverifikasi|Terjangkau/);
    expect(renderToStaticMarkup(<ConnectionAddress endpoint={{ ...endpoint, enabled: false }} />)).toContain('Dinonaktifkan');
  });
  it('provides English activity and connection labels', async () => {
    await i18n.changeLanguage('en');
    expect(renderToStaticMarkup(<JobsPage />)).toContain('Track your background operations');
    expect(renderToStaticMarkup(<ConnectivityPage />)).toContain('Connection info');
  });
});
