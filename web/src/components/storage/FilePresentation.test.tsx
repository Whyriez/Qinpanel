import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { StorageFileIcon, storageFileKind, storageFileType } from './FilePresentation';

describe('Storage file presentation', () => {
  it.each([
    ['photos/foto + café.JPEG', 'image', 'JPEG'], ['report.pdf', 'document', 'PDF'],
    ['recording.MP4', 'video', 'MP4'], ['audio.flac', 'audio', 'FLAC'],
    ['backup.tar.gz', 'archive', 'GZ'], ['config.json', 'code', 'JSON'],
    ['README', 'other', 'FILE'], ['.env', 'other', 'FILE'], ['unknown.binaryformat', 'other', 'BINARYFORM'],
  ])('classifies %s without changing its object key', (key, kind, type) => {
    expect(storageFileKind(key)).toBe(kind); expect(storageFileType(key)).toBe(type);
    const html = renderToString(<StorageFileIcon name={key} />);
    expect(html).toContain(`storage-file-glyph--${kind}`); expect(html).toContain('aria-hidden="true"');
  });
});
