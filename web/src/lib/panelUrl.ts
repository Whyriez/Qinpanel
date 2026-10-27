/** Resolve browser URLs against the current, server-validated panel address. */
export function panelHttpUrl(path: string, base = window.location.href) {
  const origin = new URL(base);
  if (!['http:', 'https:'].includes(origin.protocol) || !path.startsWith('/') || path.startsWith('//') || path.includes('\\')) throw new Error('Tautan panel tidak valid.');
  const url = new URL(path, origin);
  if (url.origin !== origin.origin || url.username || url.password) throw new Error('Tautan lintas asal ditolak.');
  return url;
}
export function panelWebSocketUrl(path: string, base = window.location.href) {
  if (!path.startsWith('/ws/')) throw new Error('Jalur WebSocket panel tidak valid.');
  const url = panelHttpUrl(path, base);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.toString();
}
