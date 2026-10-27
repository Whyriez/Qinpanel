import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FileSearchModal, SearchReportData } from '../FileSearchModal';

describe('FileSearchModal Component', () => {
  it('renders modal with search controls when open', () => {
    const html = renderToString(
      <FileSearchModal
        isOpen={true}
        onClose={() => {}}
        currentPath="sites/example.com"
      />
    );

    expect(html).toContain('Pencarian Berkas &amp; Konten');
    expect(html).toContain('Ketik nama berkas atau kata kunci yang dicari...');
    expect(html).toContain('Filter');
    expect(html).toContain('Cari');
    expect(html).toContain('Masukkan kata kunci untuk memulai pencarian');
  });

  it('renders search results with matches snippets when initialReport is provided', () => {
    const mockReport: SearchReportData = {
      items: [
        {
          path: 'public/index.php',
          isDir: false,
          sizeBytes: 2048,
          modifiedAt: 1710000000,
          matches: [
            {
              lineNumber: 14,
              lineText: "$db_pass = 'SECRET_TOKEN';",
            },
          ],
        },
        {
          path: 'public/assets',
          isDir: true,
          sizeBytes: 4096,
          modifiedAt: 1710000000,
          matches: [],
        },
      ],
      totalScanned: 150,
      limitReached: false,
      timedOut: false,
      durationMs: 42,
    };

    const html = renderToString(
      <FileSearchModal
        isOpen={true}
        onClose={() => {}}
        initialQuery="SECRET_TOKEN"
        initialReport={mockReport}
      />
    );

    expect(html).toContain('index.php');
    expect(html).toContain('public/index.php');
    expect(html).toContain('assets');
    expect(html).toContain('Baris 14:');
    expect(html).toContain('SECRET_TOKEN');
    expect(html).toContain('$db_pass');
    expect(html).toContain('Ditemukan: 2 hasil • Dipindai: 150 entri');
    expect(html).toContain('Waktu: 42 ms');
  });

  it('renders limit reached warning banner when limitReached is true', () => {
    const mockReport: SearchReportData = {
      items: [],
      totalScanned: 500,
      limitReached: true,
      timedOut: false,
      durationMs: 95,
    };

    const html = renderToString(
      <FileSearchModal
        isOpen={true}
        onClose={() => {}}
        initialQuery="test"
        initialReport={mockReport}
      />
    );

    expect(html).toContain('Batas maksimum hasil (100) tercapai');
  });

  it('renders empty search state when report items array is empty', () => {
    const mockReport: SearchReportData = {
      items: [],
      totalScanned: 80,
      limitReached: false,
      timedOut: false,
      durationMs: 15,
    };

    const html = renderToString(
      <FileSearchModal
        isOpen={true}
        onClose={() => {}}
        initialQuery="nonexistent"
        initialReport={mockReport}
      />
    );

    expect(html).toContain('Tidak ada berkas yang cocok');
    expect(html).toContain('Coba periksa ejaan atau ubah opsi filter pencarian Anda.');
  });

  it('does not render when isOpen is false', () => {
    const html = renderToString(
      <FileSearchModal
        isOpen={false}
        onClose={() => {}}
      />
    );

    expect(html).toBe('');
  });
});
