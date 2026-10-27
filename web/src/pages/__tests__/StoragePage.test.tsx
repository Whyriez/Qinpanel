import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { StoragePage } from '../StoragePage';
import i18n from '../../i18n';
import { postLoginDestination } from '../../lib/routeAccess';
vi.mock('../../context/AuthContext', () => ({ useAuth: () => ({ user: { id: 1, role: 'admin' } }) }));
describe('Storage page', () => {
  it('provides a named storage page and accessible loading without fake usage totals', () => {
    const html = renderToString(<StoragePage />); expect(html).toContain('Storage S3'); expect(html).toContain('role="status"'); expect(html).toContain('Hubungkan S3'); expect(html).not.toContain('0 B'); expect(html).not.toContain('secret_key');
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
