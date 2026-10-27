import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SecurityProfilePage } from '../SecurityProfilePage';
import { FirstRunWizardPage } from '../FirstRunWizardPage';
import '../../i18n';

vi.mock('../../context/AuthContext', () => ({
  useAuth: () => ({ user: { id: 1, username: 'admin_generated', role: 'admin', totp_enabled: false }, refreshUser: vi.fn() }),
}));

describe('Administrator access', () => {
  it('offers an accessible username and password form without prefilled secrets', () => {
    const html = renderToString(<SecurityProfilePage />);
    expect(html).toContain('Username dan kata sandi');
    expect(html).toContain('value="admin_generated"');
    for (const id of ['account-username', 'account-current-password', 'account-new-password', 'account-confirm-password']) {
      expect(html).toContain(`for="${id}"`);
      expect(html).toContain(`id="${id}"`);
    }
    expect(html).toMatch(/autocomplete="current-password"/i);
    expect(html).toMatch(/autocomplete="new-password"/i);
    expect(html).toContain('Kosongkan'.toLowerCase());
    expect(html).toContain('mencabut sesi lain');
  });

  it('keeps /setup as recovery instructions without a public account creation form', () => {
    const html = renderToString(<FirstRunWizardPage onFinish={vi.fn()} onBackToLogin={vi.fn()} />);
    expect(html).toContain('Akses administrator');
    expect(html).toContain('sudo panelctl admin list');
    expect(html).toContain('sudo panelctl admin reset-password --username NAMA');
    expect(html).toContain('sudo panelctl admin bootstrap');
    expect(html).not.toContain('<form');
    expect(html).not.toContain('Token Setup');
    expect(html).not.toContain('type="password"');
  });
});
