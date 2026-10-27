import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FileEditorModal } from '../FileEditorModal';

describe('FileEditorModal Component', () => {
  const defaultProps = {
    isOpen: true,
    onClose: () => {},
    filePath: 'sites/example.com/index.html',
    fileName: 'index.html',
    initialContent: '<!DOCTYPE html><html><body><h1>Hello World</h1></body></html>',
    initialMtime: 1710000000,
    initialEtag: '"etag-1234"',
    sizeBytes: 1024,
    onSave: async () => ({ etag: '"etag-saved"', modifiedAt: 1710000001 }),
  };

  it('renders modal with file information when open', () => {
    const html = renderToString(<FileEditorModal {...defaultProps} />);

    expect(html).toContain('index.html');
    expect(html).toContain('sites/example.com/index.html');
    expect(html).toContain('Simpan');
    expect(html).toContain('Ctrl+S');
    expect(html).toContain('Encoding: UTF-8');
    expect(html).toContain('Mode: Edit');
  });

  it('renders read-only badge and indicator when file exceeds 5MB', () => {
    const largeProps = {
      ...defaultProps,
      sizeBytes: 6 * 1024 * 1024, // 6 MB
    };

    const html = renderToString(<FileEditorModal {...largeProps} />);

    expect(html).toContain('Hanya Baca');
    expect(html).toContain('Ukuran berkas melebihi batas 5 MB');
    expect(html).toContain('Mode: Read-Only');
    expect(html).not.toContain('Simpan'); // Save button is hidden in read-only mode
  });

  it('renders read-only badge when isReadOnly is set to true', () => {
    const readOnlyProps = {
      ...defaultProps,
      isReadOnly: true,
    };

    const html = renderToString(<FileEditorModal {...readOnlyProps} />);

    expect(html).toContain('Hanya Baca');
    expect(html).toContain('Mode: Read-Only');
    expect(html).not.toContain('Simpan');
  });

  it('does not render when isOpen is false', () => {
    const html = renderToString(<FileEditorModal {...defaultProps} isOpen={false} />);
    expect(html).toBe('');
  });
});
