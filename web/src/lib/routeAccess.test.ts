import { describe, expect, it } from 'vitest';
import { isAdminOnlyRoute, postLoginDestination } from './routeAccess';

describe('Login destination and page access', () => {
  it.each(['/runtime', '/plans', '/tenants/42', '/users', '/settings', '/audit'])('tenant cannot resume admin route %s', target => {
    expect(postLoginDestination(target, 'user')).toBe('/dashboard');
  });

  it.each(['/dashboard', '/sites?status=active', '/sites/7/settings?tab=php', '/files?path=public', '/terminal?site=7', '/jobs', '/profile', '/analytics', '/analytics?range=1h#traffic'])('preserves authorized tenant route %s', target => {
    expect(postLoginDestination(target, 'user')).toBe(target);
  });

  it.each([null, '', '/', '/login', '/setup?token=fixture', 'https://other.example/runtime', '//other.example/sites', '/\\other.example/sites', '/missing', '/sites/../runtime', '/sites/../setup'])('uses the dashboard for invalid, auth-loop or unauthorized target %s', target => {
    expect(postLoginDestination(target, 'user')).toBe('/dashboard');
  });

  it('retains authorized admin destinations and query strings', () => {
    expect(postLoginDestination('/runtime?version=8.4', 'admin')).toBe('/runtime?version=8.4');
    expect(postLoginDestination('/tenants/42', 'admin')).toBe('/tenants/42');
  });

  it('uses safe permissions for an unknown role', () => {
    expect(postLoginDestination('/plans', undefined)).toBe('/dashboard');
    expect(postLoginDestination('/plans', 'reseller')).toBe('/dashboard');
  });

  it('shares the same top-level page rule without blocking tenant site settings', () => {
    expect(isAdminOnlyRoute('/settings')).toBe(true);
    expect(isAdminOnlyRoute('/tenants/42')).toBe(true);
    expect(isAdminOnlyRoute('/sites/7/settings')).toBe(false);
    expect(isAdminOnlyRoute('/dashboard')).toBe(false);
    expect(isAdminOnlyRoute('/analytics')).toBe(false);
  });
});
