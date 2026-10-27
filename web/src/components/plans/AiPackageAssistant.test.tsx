import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { AiPackageAssistant } from './AiPackageAssistant';
import i18n from '../../i18n';

describe('AI package assistant', () => {
  it('uses the accessible modal and makes clear it prepares a new draft', () => {
    const html = renderToString(<AiPackageAssistant onClose={vi.fn()} onUse={vi.fn()} />);
    expect(html).toContain('role="dialog"'); expect(html).toContain('aria-modal="true"');
    expect(html).toContain('Bantu atur dengan AI'); expect(html).toContain('draft paket baru');
    expect(html).toContain('Tutup asisten AI'); expect(html).toContain('role="status"');
  });
  it('supports English without changing the draft action scope', async () => {
    await i18n.changeLanguage('en');
    try {
      const html = renderToString(<AiPackageAssistant onClose={vi.fn()} onUse={vi.fn()} />);
      expect(html).toContain('Help size with AI'); expect(html).toContain('new hosting plan draft');
    } finally { await i18n.changeLanguage('id'); }
  });
});
