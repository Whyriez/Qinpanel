// getRandomValues also works when the panel is opened over HTTP on a LAN.
export function createSiteRuleId(kind: 'redirect' | 'header' | 'ip'): string {
  const bytes = crypto.getRandomValues(new Uint32Array(4));
  return `${kind}_${Array.from(bytes, value => value.toString(16).padStart(8, '0')).join('')}`;
}
