import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { AiSettings } from '../AiSettings';
import i18n from '../../../i18n';

describe('AiSettings', () => {
  it('presents a labelled integration panel with an accessible initial loading state', () => {
    const html = renderToString(<AiSettings />);
    expect(html).toContain('Integrasi AI'); expect(html).toContain('role="status"');
    expect(html).toContain('Gemini atau OpenAI'); expect(html).not.toContain('fixture_key');
  });
  it('uses English labels when English is active', async () => {
    await i18n.changeLanguage('en');
    try { expect(renderToString(<AiSettings />)).toContain('AI integration'); }
    finally { await i18n.changeLanguage('id'); }
  });
});
