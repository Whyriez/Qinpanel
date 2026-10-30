import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { BucketWorkspace, StoragePage } from '../StoragePage';
import type { StorageConnection } from '../../api/storage';
import i18n from '../../i18n';
import { postLoginDestination } from '../../lib/routeAccess';
vi.mock('../../context/AuthContext', () => ({ useAuth: () => ({ user: { id: 1, role: 'admin' } }) }));
describe('Storage page', () => {
  it('provides a named storage page and accessible loading without fake usage totals', () => {
    const html = renderToString(<StoragePage />); expect(html).toContain('Storage S3'); expect(html).toContain('role="status"'); expect(html).toContain('Hubungkan S3'); expect(html).not.toContain('0 B'); expect(html).not.toContain('secret_key');
  });
  it('shows administrators bucket allocations without promising access to tenant files', () => {
    const html = renderToString(<StoragePage />);
    expect(html).toContain('Bucket &amp; alokasi tenant');
    expect(html).toContain('Kelola alokasi tanpa membuka file mereka');
    expect(html).toContain('Bucket lokal dan S3 eksternal dapat aktif bersamaan');
    expect(html).toContain('tabindex="-1"');
  });
  it('never mounts the file explorer for admin or impersonated sessions', () => {
    const connection = { id: 'fixture', bucket: 'media', provider: 'compatible' } as StorageConnection;
    for (const props of [{ admin: true, impersonated: false }, { admin: false, impersonated: true }]) {
      const html = renderToString(<BucketWorkspace {...props} connection={connection} english={false} onBusy={vi.fn()} />);
      expect(html).toContain('Workspace file milik tenant');
      expect(html).not.toContain('storage-explorer');
      expect(html).not.toContain('type="file"');
    }
    const html = renderToString(<BucketWorkspace admin={false} impersonated={false} connection={connection} english={false} onBusy={vi.fn()} />);
    expect(html).toContain('storage-explorer');
    expect(html).toContain('Tampilan grid');
    expect(html).toContain('Cari di halaman ini');
  });
  it('supports English copy and scoped project access', async () => {
    await i18n.changeLanguage('en');
    try { const html = renderToString(<StoragePage scopeSiteId={4} scopeOwnerId={2} />); expect(html).toContain('Connections for this project'); expect(html).toContain('Connect S3'); }
    finally { await i18n.changeLanguage('id'); }
  });
  it('preserves the authenticated return path for both tenant and admin roles', () => {
    for (const role of ['admin', 'user']) expect(postLoginDestination('/storage?site=4', role)).toBe('/storage?site=4');
  });
});
