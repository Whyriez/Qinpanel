import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SitesPage } from '../SitesPage';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('SitesPage Component', () => {
  it('renders site management header, new site button, and search input', () => {
    const html = renderToString(
      <AuthProvider>
        <SitesPage />
      </AuthProvider>
    );

    expect(html).toContain('Situs');
    expect(html).toContain('Situs baru');
    expect(html).toContain('Cari nama domain...');
  });
});
