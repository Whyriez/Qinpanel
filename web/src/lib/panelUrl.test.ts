import { describe, expect, it } from 'vitest';
import { panelHttpUrl, panelWebSocketUrl } from './panelUrl';
describe('Panel browser URLs', () => {
  it.each([
    ['http://203.0.113.8:17890/sites', 'ws://203.0.113.8:17890/ws/terminal/session'],
    ['https://panel.example.test/dashboard', 'wss://panel.example.test/ws/terminal/session'],
    ['https://[2001:db8::7]:19443/terminal', 'wss://[2001:db8::7]:19443/ws/terminal/session'],
  ])('retains scheme, IP family and port for %s', (base, expected) => {
    expect(panelWebSocketUrl('/ws/terminal/session', base)).toBe(expected);
    expect(panelHttpUrl('/api/v1/sites/1/backups/2/download', base).origin).toBe(new URL(base).origin);
  });
  it('retains encoded log queries and reconnect URLs', () => {
    expect(panelWebSocketUrl('/ws/logs/2?type=access&lines=100', 'https://[::1]:8443/sites/2')).toBe('wss://[::1]:8443/ws/logs/2?type=access&lines=100');
  });
  it.each(['//evil.test/ws/logs/1', 'https://evil.test/ws/logs/1', '/\\evil.test/ws/logs/1', '/api/v1/sites'])('rejects invalid WebSocket target %s', path => {
    expect(() => panelWebSocketUrl(path, 'https://panel.test')).toThrow();
  });
});
