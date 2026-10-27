import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { LoginPage } from '../LoginPage';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('LoginPage Component', () => {
  it('renders login form with username, password and submit button', () => {
    const onOpenWizard = vi.fn();

    const html = renderToString(
      <AuthProvider>
        <LoginPage onOpenWizard={onOpenWizard} />
      </AuthProvider>
    );

    expect(html).toContain('Masuk');
    expect(html).toContain('Nama pengguna');
    expect(html).toContain('Kata sandi');
    expect(html).toContain('Bantuan akses admin');
    expect(html).not.toContain('Token Setup');
    expect(html).toContain('Ruang kendali untuk hosting Anda.');
    expect(html).not.toMatch(/Rust|WCAG|v0\.1\.0/);
    expect(html).toContain('for="login-username"');
    expect(html).toMatch(/autocomplete="current-password"/i);
  });
});
