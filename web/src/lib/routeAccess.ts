const adminPages = new Set(['tenants', 'users', 'plans', 'runtime', 'settings', 'audit']);
// Analytics has tenant site data; its host controls have their own admin checks.
const panelPages = new Set([...adminPages, 'dashboard', 'sites', 'files', 'storage', 'terminal', 'email', 'jobs', 'profile', 'analytics', 'connectivity']);
const panelOrigin = 'http://panel.local';

/** Match the top-level page, so site settings remain a tenant route. */
export function isAdminOnlyRoute(pathname: string): boolean {
  return adminPages.has(pathname.split('/')[1]);
}

/** A returnTo from an old account must be allowed for the newly signed-in user. */
export function postLoginDestination(returnTo: string | null, role: string | undefined): string {
  if (!returnTo?.startsWith('/') || returnTo.startsWith('//') || returnTo.includes('\\')) return '/dashboard';
  try {
    const target = new URL(returnTo, panelOrigin);
    if (target.origin !== panelOrigin || !panelPages.has(target.pathname.split('/')[1])) return '/dashboard';
    if (role !== 'admin' && isAdminOnlyRoute(target.pathname)) return '/dashboard';
    return target.pathname + target.search + target.hash;
  } catch {
    return '/dashboard';
  }
}
