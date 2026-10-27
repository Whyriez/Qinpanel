import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { PlansPage } from '../PlansPage';
import '../../i18n';

describe('PlansPage Component', () => {
  it('renders hosting plans header and create button', () => {
    const html = renderToString(<PlansPage />);

    expect(html).toContain('Paket Hosting');
    expect(html).toContain('Buat Paket Baru');
  });
});
