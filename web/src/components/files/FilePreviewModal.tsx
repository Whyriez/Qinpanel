import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useId, useCallback } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import {
  X,
  Download,
  Edit3,
  ZoomIn,
  ZoomOut,
  RotateCw,
  Maximize2,
  FileText,
  Image as ImageIcon,
  Film,
  Music,
  File,
  AlertCircle,
} from 'lucide-react';

export interface FilePreviewModalProps {
  isOpen: boolean;
  onClose: () => void;
  filePath: string;
  fileName: string;
  sizeBytes: number;
  mimeType?: string;
  textContent?: string;
  onOpenEditor?: () => void;
  apiBaseUrl?: string;
}

export type PreviewType = 'image' | 'pdf' | 'video' | 'audio' | 'text' | 'unsupported';

export function detectPreviewType(fileName: string, mime?: string): PreviewType {
  const ext = fileName.split('.').pop()?.toLowerCase() || '';

  if (
    ['jpg', 'jpeg', 'png', 'gif', 'svg', 'webp', 'bmp', 'ico'].includes(ext) ||
    mime?.startsWith('image/')
  ) {
    return 'image';
  }

  if (ext === 'pdf' || mime === 'application/pdf') {
    return 'pdf';
  }

  if (
    ['mp4', 'webm', 'ogg', 'mov', 'mkv'].includes(ext) ||
    mime?.startsWith('video/')
  ) {
    return 'video';
  }

  if (
    ['mp3', 'wav', 'ogg', 'm4a', 'aac', 'flac'].includes(ext) ||
    mime?.startsWith('audio/')
  ) {
    return 'audio';
  }

  if (
    [
      'txt',
      'log',
      'md',
      'json',
      'js',
      'ts',
      'jsx',
      'tsx',
      'html',
      'htm',
      'css',
      'scss',
      'php',
      'py',
      'rs',
      'sh',
      'bash',
      'yml',
      'yaml',
      'toml',
      'env',
      'ini',
      'conf',
      'sql',
      'xml',
      'c',
      'cpp',
      'h',
      'java',
      'go',
    ].includes(ext) ||
    mime?.startsWith('text/') ||
    mime === 'application/json' ||
    mime === 'application/javascript'
  ) {
    return 'text';
  }

  return 'unsupported';
}

export const FilePreviewModal: React.FC<FilePreviewModalProps> = ({
  isOpen,
  onClose,
  filePath,
  fileName,
  sizeBytes,
  mimeType,
  textContent,
  onOpenEditor,
  apiBaseUrl = '/api/v1/files/download',
}) => {
  const modalId = useId();
  const [zoom, setZoom] = useState(1);
  const [rotation, setRotation] = useState(0);
  const [imageDimensions, setImageDimensions] = useState<{ width: number; height: number } | null>(
    null
  );

  const previewType = detectPreviewType(fileName, mimeType);
  const downloadUrl = `${apiBaseUrl}${apiBaseUrl.includes('?') ? '&' : '?'}path=${encodeURIComponent(filePath)}`;

  // Reset zoom & rotation when file changes
  useEffect(() => {
    setZoom(1);
    setRotation(0);
    setImageDimensions(null);
  }, [filePath, isOpen]);

  // Handle escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!isOpen) return;
      if (e.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  const handleZoomIn = useCallback(() => {
    setZoom((prev) => Math.min(prev + 0.25, 4));
  }, []);

  const handleZoomOut = useCallback(() => {
    setZoom((prev) => Math.max(prev - 0.25, 0.25));
  }, []);

  const handleResetZoom = useCallback(() => {
    setZoom(1);
    setRotation(0);
  }, []);

  const handleRotate = useCallback(() => {
    setRotation((prev) => (prev + 90) % 360);
  }, []);

  if (!isOpen) return null;

  const formatSize = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  };

  const getHeaderIcon = () => {
    switch (previewType) {
      case 'image':
        return <ImageIcon className="h-4 w-4" />;
      case 'pdf':
      case 'text':
        return <FileText className="h-4 w-4" />;
      case 'video':
        return <Film className="h-4 w-4" />;
      case 'audio':
        return <Music className="h-4 w-4" />;
      default:
        return <File className="h-4 w-4" />;
    }
  };

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-2 sm:p-4 bg-black/75 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
      aria-labelledby={`${modalId}-title`}
      onDismiss={onClose}
    >
      <div className="flex flex-col w-full max-w-5xl h-[90vh] rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-floating)] animate-in fade-in zoom-in-95 duration-150 text-[var(--foreground)] overflow-hidden dialog-panel">
        {/* Header Bar */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--line)] bg-[var(--surface)] dialog-header">
          <div className="flex items-center gap-3 min-w-0">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-blue-500/10 text-blue-500">
              {getHeaderIcon()}
            </div>
            <div className="min-w-0">
              <h3 id={`${modalId}-title`} className="text-sm font-semibold truncate text-[var(--foreground)]">
                {fileName}
              </h3>
              <p className="text-[11px] text-[var(--muted-foreground)] truncate max-w-md" title={filePath}>
                {filePath} • {formatSize(sizeBytes)}
              </p>
            </div>
          </div>

          {/* Action Bar */}
          <div className="flex items-center gap-2 shrink-0">
            {/* Image Controls */}
            {previewType === 'image' && (
              <div className="hidden sm:flex items-center gap-1 border-r border-[var(--line)] pr-2 mr-1">
                <button
                  type="button"
                  onClick={handleZoomOut}
                  className="p-1.5 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)] transition-colors"
                  title="Perkecil (-)"
                  aria-label="Perkecil"
                >
                  <ZoomOut className="h-4 w-4" />
                </button>
                <span className="text-[11px] font-mono text-[var(--muted-foreground)] w-10 text-center">
                  {Math.round(zoom * 100)}%
                </span>
                <button
                  type="button"
                  onClick={handleZoomIn}
                  className="p-1.5 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)] transition-colors"
                  title="Perbesar (+)"
                  aria-label="Perbesar"
                >
                  <ZoomIn className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  onClick={handleRotate}
                  className="p-1.5 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)] transition-colors"
                  title="Putar 90°"
                  aria-label="Putar"
                >
                  <RotateCw className="h-4 w-4" />
                </button>
                <button
                  type="button"
                  onClick={handleResetZoom}
                  className="p-1.5 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)] transition-colors"
                  title="Atur Ulang Tampilan"
                  aria-label="Atur Ulang Tampilan"
                >
                  <Maximize2 className="h-4 w-4" />
                </button>
              </div>
            )}

            {/* Open in Editor button if text */}
            {previewType === 'text' && onOpenEditor && (
              <button
                type="button"
                onClick={onOpenEditor}
                className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-blue-600/10 text-blue-500 hover:bg-blue-600/20 border border-blue-500/20 transition-colors"
              >
                <Edit3 className="h-3.5 w-3.5" />
                <span>Buka di Editor</span>
              </button>
            )}

            {/* Download Button */}
            <a
              href={downloadUrl}
              download={fileName}
              className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--surface-hover)] hover:bg-[var(--line)] text-[var(--foreground)] transition-colors"
              title="Unduh Berkas"
            >
              <Download className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">Unduh</span>
            </a>

            {/* Close Button */}
            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)]"
              aria-label="Tutup Pratinjau"
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        </div>

        {/* Content Body */}
        <div className="flex-1 overflow-auto relative bg-[var(--background)] flex items-center justify-center p-0 dialog-body">
          {/* 1. Image Preview */}
          {previewType === 'image' && (
            <div className="w-full h-full overflow-auto flex items-center justify-center p-4 checkerboard-pattern select-none">
              <img
                src={downloadUrl}
                alt={fileName}
                style={{
                  transform: `scale(${zoom}) rotate(${rotation}deg)`,
                  transition: 'transform 0.15s ease-out',
                }}
                onLoad={(e) => {
                  const target = e.currentTarget;
                  setImageDimensions({ width: target.naturalWidth, height: target.naturalHeight });
                }}
                className="max-h-[80vh] max-w-full object-contain rounded-[var(--radius-sm)] shadow-md"
              />
            </div>
          )}

          {/* 2. PDF Preview */}
          {previewType === 'pdf' && (
            <div className="w-full h-full">
              <object
                data={`${downloadUrl}#toolbar=1`}
                type="application/pdf"
                className="w-full h-full"
                aria-label="PDF Document Viewer"
              >
                <div className="flex flex-col items-center justify-center h-full p-6 text-center text-sm text-[var(--muted-foreground)]">
                  <FileText className="h-12 w-12 text-[var(--muted-foreground)] mb-3" />
                  <p className="font-medium text-[var(--foreground)] mb-1">
                    Pratinjau PDF tidak didukung langsung oleh browser Anda.
                  </p>
                  <p className="mb-4">Anda dapat mengunduh berkas untuk membukanya secara terpisah.</p>
                  <a
                    href={downloadUrl}
                    download={fileName}
                    className="flex items-center gap-2 px-4 py-2 bg-blue-600 text-white rounded-[var(--radius-md)] text-xs font-medium hover:bg-blue-700"
                  >
                    <Download className="h-4 w-4" />
                    Unduh Berkas PDF
                  </a>
                </div>
              </object>
            </div>
          )}

          {/* 3. Video Preview */}
          {previewType === 'video' && (
            <div className="w-full h-full flex items-center justify-center p-4 bg-black/90">
              <video
                controls
                controlsList="nodownload"
                className="max-h-full max-w-full rounded-[var(--radius-md)] shadow-lg"
              >
                <source src={downloadUrl} />
                Browser Anda tidak mendukung pemutaran video ini.
              </video>
            </div>
          )}

          {/* 4. Audio Preview */}
          {previewType === 'audio' && (
            <div className="w-full h-full flex flex-col items-center justify-center p-8 bg-[var(--surface)] max-w-lg mx-auto">
              <div className="flex h-20 w-20 items-center justify-center rounded-full bg-blue-500/10 text-blue-500 mb-6 shadow-inner">
                <Music className="h-10 w-10 animate-pulse" />
              </div>
              <h4 className="text-base font-semibold mb-1 text-center text-[var(--foreground)] truncate max-w-xs">
                {fileName}
              </h4>
              <p className="text-xs text-[var(--muted-foreground)] mb-6">
                Audio Stream ({formatSize(sizeBytes)})
              </p>
              <audio controls className="w-full rounded-md shadow-sm">
                <source src={downloadUrl} />
                Browser Anda tidak mendukung pemutar audio ini.
              </audio>
            </div>
          )}

          {/* 5. Text / Code Preview */}
          {previewType === 'text' && (
            <div className="w-full h-full overflow-auto">
              {textContent !== undefined ? (
                <CodeMirror
                  value={textContent}
                  height="100%"
                  theme="dark"
                  readOnly={true}
                  editable={false}
                  basicSetup={{
                    lineNumbers: true,
                    foldGutter: true,
                    highlightActiveLine: false,
                  }}
                  className="h-full text-sm font-mono"
                />
              ) : (
                <div className="flex items-center justify-center h-full text-xs text-[var(--muted-foreground)]">
                  Memuat isi teks...
                </div>
              )}
            </div>
          )}

          {/* 6. Unsupported / Binary Fallback */}
          {previewType === 'unsupported' && (
            <div className="flex flex-col items-center justify-center h-full p-6 text-center max-w-md">
              <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-zinc-500/10 text-zinc-400 mb-4">
                <AlertCircle className="h-8 w-8" />
              </div>
              <h4 className="text-base font-semibold text-[var(--foreground)] mb-1">Pratinjau Tidak Tersedia</h4>
              <p className="text-xs text-[var(--muted-foreground)] mb-5">
                Berkas ini berupa biner atau format yang tidak didukung untuk pratinjau langsung di dalam browser.
                Silakan unduh berkas untuk melihat isinya.
              </p>
              <a
                href={downloadUrl}
                download={fileName}
                className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-[var(--radius-md)] text-xs font-medium shadow-sm transition-colors"
              >
                <Download className="h-4 w-4" />
                {`Unduh Berkas (${formatSize(sizeBytes)})`}
              </a>
            </div>
          )}
        </div>

        {/* Footer info bar */}
        <div className="flex items-center justify-between px-4 py-2 border-t border-[var(--line)] bg-[var(--surface)] text-[11px] text-[var(--muted-foreground)] dialog-footer">
          <div>
            Tipe: <span className="font-mono uppercase">{previewType.toUpperCase()}</span>
            {imageDimensions && (
              <span className="ml-3 font-mono">
                {`Dimensi: ${imageDimensions.width} × ${imageDimensions.height} px`}
              </span>
            )}
          </div>
          <div>{`Ukuran: ${formatSize(sizeBytes)}`}</div>
        </div>
      </div>
    </ModalOverlay>
  );
};
