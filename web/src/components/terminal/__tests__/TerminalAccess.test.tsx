import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { TerminalAccessNotice, type TerminalAccessState } from '../TerminalAccess';

const auth = vi.hoisted(() => ({ role: 'admin' }));
vi.mock('../../../context/AuthContext', () => ({
  useAuth: () => ({ user: { id: 42, role: auth.role } }),
}));

const blocked = (effectivePolicy: string, inherited = true): TerminalAccessState => ({
  status: 'blocked',
  policy: {
    user_id: 12,
    username: 'alice',
    shell_policy: inherited ? null : effectivePolicy,
    effective_policy: effectivePolicy,
    inherited_from_plan: inherited,
    plan_policy: inherited ? effectivePolicy : 'full',
  },
});
const render = (state: TerminalAccessState) =>
  renderToString(<TerminalAccessNotice state={state} onRefresh={vi.fn()} />);

afterEach(() => {
  auth.role = 'admin';
});

describe('TerminalAccessNotice', () => {
  it('explains SFTP access and links administrators to the selected tenant settings', () => {
    const html = render(blocked('sftp'));
    expect(html).toContain('mode SFTP saja');
    expect(html).toContain('Mengikuti paket');
    expect(html).toContain('/tenants/12/overview?section=shell');
    expect(html).toContain('Web Terminal dan SSH interaktif');
  });

  it('shows a tenant override even when the plan allows full shell', () => {
    const html = render(blocked('none', false));
    expect(html).toContain('Pengaturan khusus tenant');
    expect(html).toContain('Terminal belum diaktifkan');
    expect(html).not.toContain('mode SFTP saja');
  });

  it('directs regular tenants to their administrator without offering policy changes', () => {
    auth.role = 'user';
    const html = render(blocked('none'));
    expect(html).toContain('Hubungi administrator');
    expect(html).not.toContain('Atur akses terminal');
    expect(html).not.toContain('/tenants/');
    expect(html).toContain('Periksa ulang akses');
  });

  it('keeps loading and failed checks distinct from denied permissions', () => {
    expect(render({ status: 'loading' })).toContain('Memeriksa kebijakan');
    const html = render({ status: 'error', error: 'Server belum tersedia' });
    expect(html).toContain('Server belum tersedia');
    expect(html).toContain('role="alert"');
    expect(html).not.toContain('Atur akses terminal');
  });

  it('does not show a denial when full access is allowed', () => {
    expect(render({ status: 'allowed' })).toBe('');
  });
});
