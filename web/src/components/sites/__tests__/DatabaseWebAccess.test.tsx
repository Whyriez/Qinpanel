import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { DatabaseWebAccessButton } from '../DatabaseWebAccessButton';
import { DatabaseWebToolsCard } from '../../runtime/DatabaseWebToolsCard';
import '../../../i18n';

describe('Internal database access', () => {
  it('names phpMyAdmin and announces the database and new tab for MySQL', () => {
    const html = renderToString(<DatabaseWebAccessButton id={10} name="p_alice_shop" engine="mariadb" />);
    expect(html).toContain('Buka phpMyAdmin');
    expect(html).toContain('p_alice_shop di tab baru');
    expect(html).not.toContain('password');
    expect(html).not.toContain('3306');
  });
  it('names Adminer for PostgreSQL', () => {
    const html = renderToString(<DatabaseWebAccessButton id={11} name="p_alice_app" engine="postgresql" />);
    expect(html).toContain('Buka Adminer');
    expect(html).toContain('p_alice_app di tab baru');
  });
  it('holds installation until inventory and active jobs have been checked', () => {
    const html = renderToString(<DatabaseWebToolsCard />);
    expect(html).toContain('Pengelola database internal');
    expect(html).toContain('Memeriksa pengelola');
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>[\s\S]*Pasang pengelola database/);
  });
});
