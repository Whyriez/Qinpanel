import { File, FileArchive, FileAudio, FileCode2, FileText, FileVideo, Image } from 'lucide-react';

export type StorageFileKind = 'image' | 'document' | 'video' | 'audio' | 'archive' | 'code' | 'other';
export function storageFileKind(key: string): StorageFileKind {
  const ext = key.split('.').pop()?.toLowerCase() ?? '';
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'avif', 'heic', 'bmp', 'tiff'].includes(ext)) return 'image';
  if (['pdf', 'doc', 'docx', 'xls', 'xlsx', 'csv', 'ppt', 'pptx', 'txt', 'md', 'odt'].includes(ext)) return 'document';
  if (['mp4', 'webm', 'mov', 'mkv', 'avi', 'm4v'].includes(ext)) return 'video';
  if (['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a'].includes(ext)) return 'audio';
  if (['zip', 'rar', '7z', 'gz', 'tar', 'bz2'].includes(ext)) return 'archive';
  if (['js', 'ts', 'tsx', 'jsx', 'html', 'css', 'json', 'xml', 'py', 'php', 'rs', 'yml', 'yaml', 'sql'].includes(ext)) return 'code';
  return 'other';
}
export function storageFileType(key: string): string {
  const filename = key.split('/').pop() ?? key;
  const dot = filename.lastIndexOf('.');
  return dot > 0 && dot < filename.length - 1 ? filename.slice(dot + 1).slice(0, 10).toUpperCase() : 'FILE';
}
export function StorageFileIcon({ name, size = 24 }: { name: string; size?: number }) {
  const kind = storageFileKind(name);
  const Icon = { image: Image, document: FileText, video: FileVideo, audio: FileAudio, archive: FileArchive, code: FileCode2, other: File }[kind];
  return <Icon size={size} aria-hidden="true" className={`storage-file-glyph storage-file-glyph--${kind}`} />;
}
