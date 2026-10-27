import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteDetailPage } from '../SiteDetailPage';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('SiteDetailPage Component', () => {
  it('renders site detail initial loading state properly', () => {
    const html = renderToString(
      <AuthProvider>
        <SiteDetailPage siteId={1} onBack={() => {}} />
      </AuthProvider>
    );

    expect(html).toContain('Memuat detail situs...');
  });
});
