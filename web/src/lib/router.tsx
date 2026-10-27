import React, { useSyncExternalStore } from 'react';
export const pagePath = (page: string) => page === 'users' ? '/tenants' : `/${page}`;
const subscribe = (listener: () => void) => {
  window.addEventListener('popstate', listener);
  return () => window.removeEventListener('popstate', listener);
};
export function useBrowserRoute() {
  const url = useSyncExternalStore(subscribe, () => window.location.pathname + window.location.search, () => '/dashboard');
  const parsed = React.useMemo(() => new URL(url, 'http://panel.local'), [url]);
  return { pathname: parsed.pathname, search: parsed.searchParams, url };
}
export function navigate(to: string, replace = false) {
  if (!to.startsWith('/') || to.startsWith('//')) throw new Error('Invalid panel route');
  if (window.location.pathname + window.location.search === to) return;
  window.history[replace ? 'replaceState' : 'pushState'](null, '', to);
  window.dispatchEvent(new PopStateEvent('popstate'));
}
export function RouteLink({ href, onClick, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement> & {href: string}) {
  return <a {...props} href={href} onClick={(event) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || props.target === '_blank') return;
    event.preventDefault(); onClick?.(event); navigate(href);
  }} />;
}
