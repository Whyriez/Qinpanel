import { describe, expect, it } from 'vitest';
import { filterMenuCommands } from '../commandFilter';

describe('Command search relevance', () => {
  it('finds a translated label regardless of case and surrounding whitespace', () => {
    expect(filterMenuCommands('page-analytics', ' ANALITIK ', ['Analitik'])).toBe(1);
  });
  it('does not suggest unrelated pages when looking for a theme', () => {
    expect(filterMenuCommands('theme-dark', 'gelap', ['Tema: Gelap'])).toBeGreaterThan(0);
    expect(filterMenuCommands('page-analytics', 'gelap', ['Analitik', 'Lalu lintas dan performa situs'])).toBe(0);
    expect(filterMenuCommands('page-profile', 'gelap', ['Profil dan keamanan', 'Profil, kata sandi, dan autentikasi'])).toBe(0);
  });
  it('supports description words in either order and ignores accents', () => {
    expect(filterMenuCommands('page-runtime', 'node php', ['Versi PHP dan Node.js'])).toBeGreaterThan(0);
    expect(filterMenuCommands('preferences', 'preferensi', ['Préferénsi'])).toBe(1);
  });
  it('prefers exact labels over description matches and returns no match for unknown text', () => {
    expect(filterMenuCommands('page-files', 'berkas', ['Berkas'])).toBeGreaterThan(filterMenuCommands('page-files', 'hosting', ['Jelajahi berkas hosting']));
    expect(filterMenuCommands('logout', 'zzzz', ['Keluar'])).toBe(0);
    expect(filterMenuCommands('page-sites', '   ', ['Situs'])).toBe(1);
  });
});
