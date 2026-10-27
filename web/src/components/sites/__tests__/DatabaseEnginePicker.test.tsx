import { describe, expect, it, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { DatabaseEnginePicker } from '../DatabaseEnginePicker';

describe('Live database engine choices', () => {
  const props = { selected: '', loading: false, error: '', onSelect: vi.fn(), onRefresh: vi.fn() };
  it('renders only backend capabilities without built-in fallback options', () => {
    const html = renderToString(<DatabaseEnginePicker {...props} engines={[{ engine: 'postgresql', label: 'PostgreSQL', port: 5432 }]} selected="postgresql" />);
    expect(html).toContain('PostgreSQL');
    expect(html).toContain('type="radio"');
    expect(html).toContain('checked=""');
    expect(html).not.toContain('MariaDB');
  });
  it('uses metadata rather than engine-specific JSX for future capabilities', () => {
    const html = renderToString(<DatabaseEnginePicker {...props} engines={[{ engine: 'future_engine', label: 'Future engine fixture', port: 1433 }]} />);
    expect(html).toContain('Future engine fixture');
    expect(html).toContain('1433');
  });
  it('keeps empty, loading and failure states distinct and offers refresh', () => {
    const empty = renderToString(<DatabaseEnginePicker {...props} engines={[]} />);
    expect(empty).toContain('Minta administrator');
    expect(empty).not.toContain('type="radio"');
    const loading = renderToString(<DatabaseEnginePicker {...props} engines={[]} loading />);
    expect(loading).toContain('Memeriksa mesin');
    expect(loading).not.toContain('Minta administrator');
    const error = renderToString(<DatabaseEnginePicker {...props} engines={[]} error="Agent tidak tersedia" />);
    expect(error).toContain('role="alert"');
    expect(error).toContain('Agent tidak tersedia');
    expect(error).toContain('Muat ulang pilihan mesin');
  });
});
