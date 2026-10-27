import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FilePreviewModal, detectPreviewType } from '../FilePreviewModal';

describe('FilePreviewModal Component & detectPreviewType', () => {
  describe('detectPreviewType', () => {
    it('detects images correctly', () => {
      expect(detectPreviewType('photo.png')).toBe('image');
      expect(detectPreviewType('logo.svg')).toBe('image');
      expect(detectPreviewType('hero.jpg')).toBe('image');
      expect(detectPreviewType('banner.webp')).toBe('image');
    });

    it('detects PDFs correctly', () => {
      expect(detectPreviewType('invoice.pdf')).toBe('pdf');
      expect(detectPreviewType('document', 'application/pdf')).toBe('pdf');
    });

    it('detects video correctly', () => {
      expect(detectPreviewType('clip.mp4')).toBe('video');
      expect(detectPreviewType('trailer.webm')).toBe('video');
    });

    it('detects audio correctly', () => {
      expect(detectPreviewType('podcast.mp3')).toBe('audio');
      expect(detectPreviewType('sound.wav')).toBe('audio');
    });

    it('detects text and code files correctly', () => {
      expect(detectPreviewType('main.rs')).toBe('text');
      expect(detectPreviewType('app.tsx')).toBe('text');
      expect(detectPreviewType('index.html')).toBe('text');
      expect(detectPreviewType('style.css')).toBe('text');
      expect(detectPreviewType('config.json')).toBe('text');
      expect(detectPreviewType('notes.txt')).toBe('text');
    });

    it('detects binary and unsupported files as unsupported', () => {
      expect(detectPreviewType('archive.zip')).toBe('unsupported');
      expect(detectPreviewType('binary.bin')).toBe('unsupported');
      expect(detectPreviewType('backup.tar.gz')).toBe('unsupported');
    });
  });

  describe('Modal Rendering', () => {
    it('renders image preview modal with zoom/rotate controls', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={true}
          onClose={() => {}}
          filePath="sites/example.com/logo.png"
          fileName="logo.png"
          sizeBytes={45000}
        />
      );

      expect(html).toContain('logo.png');
      expect(html).toContain('43.9 KB');
      expect(html).toContain('Perbesar');
      expect(html).toContain('Perkecil');
      expect(html).toContain('Putar');
      expect(html).toContain('Tipe: <span class="font-mono uppercase">IMAGE</span>');
    });

    it('renders PDF preview modal with object tag', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={true}
          onClose={() => {}}
          filePath="sites/example.com/manual.pdf"
          fileName="manual.pdf"
          sizeBytes={1024 * 1024}
        />
      );

      expect(html).toContain('manual.pdf');
      expect(html).toContain('type="application/pdf"');
      expect(html).toContain('Unduh Berkas PDF');
      expect(html).toContain('Tipe: <span class="font-mono uppercase">PDF</span>');
    });

    it('renders video player for mp4 file', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={true}
          onClose={() => {}}
          filePath="sites/example.com/intro.mp4"
          fileName="intro.mp4"
          sizeBytes={5 * 1024 * 1024}
        />
      );

      expect(html).toContain('intro.mp4');
      expect(html).toContain('<video');
      expect(html).toContain('controls=""');
      expect(html).toContain('Tipe: <span class="font-mono uppercase">VIDEO</span>');
    });

    it('renders audio player for mp3 file', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={true}
          onClose={() => {}}
          filePath="sites/example.com/jingle.mp3"
          fileName="jingle.mp3"
          sizeBytes={2 * 1024 * 1024}
        />
      );

      expect(html).toContain('jingle.mp3');
      expect(html).toContain('<audio');
      expect(html).toContain('Audio Stream');
      expect(html).toContain('Tipe: <span class="font-mono uppercase">AUDIO</span>');
    });

    it('renders text preview modal with open in editor button', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={true}
          onClose={() => {}}
          filePath="sites/example.com/index.php"
          fileName="index.php"
          sizeBytes={2048}
          textContent="<?php echo 'Hello World'; ?>"
          onOpenEditor={() => {}}
        />
      );

      expect(html).toContain('index.php');
      expect(html).toContain('Buka di Editor');
      expect(html).toContain('Tipe: <span class="font-mono uppercase">TEXT</span>');
    });

    it('renders unsupported fallback for binary file', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={true}
          onClose={() => {}}
          filePath="sites/example.com/backup.zip"
          fileName="backup.zip"
          sizeBytes={15 * 1024 * 1024}
        />
      );

      expect(html).toContain('Pratinjau Tidak Tersedia');
      expect(html).toContain('Berkas ini berupa biner atau format yang tidak didukung');
      expect(html).toContain('Unduh Berkas (15.00 MB)');
    });

    it('does not render when isOpen is false', () => {
      const html = renderToString(
        <FilePreviewModal
          isOpen={false}
          onClose={() => {}}
          filePath="sites/example.com/logo.png"
          fileName="logo.png"
          sizeBytes={1024}
        />
      );

      expect(html).toBe('');
    });
  });
});
