import { describe, expect, it } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FileEditorWorkspace, isEditorPath } from '../FileEditorWorkspace';

describe('FileEditorWorkspace', () => {
  it('renders one dialog with a directory explorer and the first open document', () => {
    const html = renderToString(<FileEditorWorkspace initialDocument={{ filePath: 'sites/example/index.html', fileName: 'index.html', initialContent: 'hello', sizeBytes: 5, etag: '"v1"' }} tenant="42" onClose={() => {}} onSaved={() => {}} />);
    expect(html.match(/role="dialog"/g)).toHaveLength(1);
    expect(html).toContain('Editor berkas');
    expect(html).toContain('Folder dan berkas');
    expect(html).toContain('role="tablist"');
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('role="tabpanel"');
    expect(html).toContain('sites/example/index.html');
    expect(html).toContain('Tutup berkas');
    expect(html).toContain('Tutup Editor');
  });

  it.each([
    ['', '', true],
    ['sites/example/index.html', '', true],
    ['sites/example/日本語 file.txt', 'sites/example', true],
    ['sites/example', 'sites/example', true],
    ['sites/example2/file.txt', 'sites/example', false],
    ['elsewhere/file.txt', 'sites/example', false],
    ['/etc/passwd', '', false],
    ['../secret', '', false],
    ['sites/../secret', '', false],
    ['sites/./file', '', false],
    ['sites//file', '', false],
    ['file\0.txt', '', false],
  ])('constrains sidebar navigation for %j in %j', (path, root, allowed) => {
    expect(isEditorPath(path, root)).toBe(allowed);
  });
});
