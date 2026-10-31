import { useTranslation } from 'react-i18next';
import { FileBrowserListing, FileInspector, fileSize } from './FileBrowserView';
import '../../styles/file-browser.css';
import { ModalOverlay } from '../ui/ModalOverlay';
import React, {
  useState,
  useEffect,
  useCallback,
  useRef,
  useId,
} from 'react';
import {
  Folder,
  FolderPlus,
  FilePlus,
  Upload,
  Search,
  RefreshCw,
  LayoutList,
  LayoutGrid,
  Trash2,
  Archive,
  Terminal,
  ChevronRight,
  Edit2,
  Eye,
  Download,
  Copy,
  Scissors,
  ClipboardPaste,
  ShieldAlert,
  ChevronDown,
  HardDrive,
  FileCode,
  FileText,
  FileArchive,
  Image,
  Film,
  Music,
  File,
  X,
  AlertTriangle,
  RotateCcw,
  Key,
  PanelLeft,
  ArrowUp,
  Info,
  EyeOff,
  MoreHorizontal,
} from 'lucide-react';
import { FsEntry, ViewMode, SortField, SortDirection, TrashedItem, UploadItem, DiskQuotaStatus } from '../../types/files';
import { createFileClient } from '../../api/files';
import { FileEditorWorkspace, type EditorDocument } from './FileEditorWorkspace';
import { FilePreviewModal } from './FilePreviewModal';
import { FilePermissionModal } from './FilePermissionModal';
import { FileSearchModal } from './FileSearchModal';
import { TerminalDrawer } from '../terminal/TerminalDrawer';
import { SshConnectModal } from '../ssh/SshConnectModal';

export interface FileManagerProps {
  tenant?: string;
  siteId?: number;
  initialPath?: string;
  baseRootPath?: string;
  title?: string;
  onNavigateTerminal?: (currentPath: string) => void;
  quotaInfo?: DiskQuotaStatus;
}

export const FileManager: React.FC<FileManagerProps> = ({
  tenant,
  siteId,
  initialPath = '',
  baseRootPath = '',
  title = 'Pengelola Berkas',
  quotaInfo,
  onNavigateTerminal,
}) => {
  const { i18n } = useTranslation();
  const formId = useId();
  const english = i18n.language.startsWith('en');
  const text = (id: string, en: string) => english ? en : id;
  const [showDetails, setShowDetails] = useState(true);
  const [mobileDetails, setMobileDetails] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [totalEntries, setTotalEntries] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const listingSequence = useRef(0);
  const fileApi = React.useMemo(() => createFileClient(tenant), [tenant]);
  const { fetchFileList, createFolder, renameFile, moveFile, copyFile, moveToTrash, fetchTrashList, restoreTrash, emptyTrash, compressArchive, extractArchive, initUpload, uploadChunk } = fileApi;
  // Navigation and files state
  const [currentPath, setCurrentPath] = useState(initialPath || baseRootPath);
  const [entries, setEntries] = useState<FsEntry[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // View, Sorting and Filter state
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [sortBy, setSortBy] = useState<SortField>('name');
  const [sortOrder, setSortOrder] = useState<SortDirection>('asc');
  const [searchFilter, setSearchFilter] = useState('');
  const [showHidden, setShowHidden] = useState(false);

  // Selection state
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [lastSelectedIndex, setLastSelectedIndex] = useState<number | null>(null);

  // Tree directory state (Left Sidebar)
  const [isTreeCollapsed, setIsTreeCollapsed] = useState(() => typeof window !== 'undefined' && window.matchMedia('(max-width: 760px)').matches);
  useEffect(() => {
    const media = window.matchMedia('(max-width: 760px)');
    const collapse = () => { if (media.matches) setIsTreeCollapsed(true); };
    media.addEventListener('change', collapse);
    return () => media.removeEventListener('change', collapse);
  }, []);
  // Internal File Clipboard (Copy / Cut)
  const [clipboard, setClipboard] = useState<{
    paths: string[];
    operation: 'copy' | 'cut';
  } | null>(null);

  // Upload Tray State
  const [uploadQueue, setUploadQueue] = useState<(UploadItem & { scope?: string })[]>([]);
  const uploadQueueRef = useRef(uploadQueue);
  useEffect(() => { uploadQueueRef.current = uploadQueue; }, [uploadQueue]);
  useEffect(() => { setUploadQueue([]); }, [tenant]);
  const [isUploadTrayMinimized, setIsUploadTrayMinimized] = useState(false);
  const [isDragOverDropzone, setIsDragOverDropzone] = useState(false);

  // Trash Drawer State
  const [isTrashOpen, setIsTrashOpen] = useState(false);
  const [trashItems, setTrashItems] = useState<TrashedItem[]>([]);
  const [isLoadingTrash, setIsLoadingTrash] = useState(false);

  // Context Menu State
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    targetEntry?: FsEntry;
  } | null>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);
  const contextReturnFocus = useRef<HTMLElement | null>(null);
  const toolsRef = useRef<HTMLDivElement>(null);

  // Modal Dialogs State
  const [isNewFolderOpen, setIsNewFolderOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');

  const [isNewFileOpen, setIsNewFileOpen] = useState(false);
  const [newFileName, setNewFileName] = useState('');

  const [isRenameOpen, setIsRenameOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<FsEntry | null>(null);
  const [renameNewName, setRenameNewName] = useState('');

  const [isCompressOpen, setIsCompressOpen] = useState(false);
  const [compressName, setCompressName] = useState('archive.zip');
  const [compressFormat, setCompressFormat] = useState<'zip' | 'tar_gz'>('zip');

  const [isDeleteConfirmOpen, setIsDeleteConfirmOpen] = useState(false);

  // Integration Modals from P5-T6, P5-T7, P5-T8
  interface ActiveEditorState extends EditorDocument { scope?: string }
  const [activeEditor, setActiveEditor] = useState<ActiveEditorState | null>(null);

  interface ActivePreviewState {
    filePath: string;
    fileName: string;
    sizeBytes: number;
  }
  const [activePreview, setActivePreview] = useState<ActivePreviewState | null>(null);

  interface ActivePermissionState {
    name: string;
    path: string;
    isDir: boolean;
    modeOctal: string;
    owner?: string;
    group?: string;
  }
  const [activePermission, setActivePermission] = useState<ActivePermissionState | null>(null);

  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [isTerminalOpen, setIsTerminalOpen] = useState(false);
  const [terminalCwd, setTerminalCwd] = useState(currentPath);
  const [terminalOpenRequest, setTerminalOpenRequest] = useState(0);
  const terminalTrigger = useRef<HTMLButtonElement>(null);
  const [isSshModalOpen, setIsSshModalOpen] = useState(false);
  const [activeSshPath, setActiveSshPath] = useState(currentPath);

  // Rubber Band Marquee State
  const fileContainerRef = useRef<HTMLDivElement>(null);
  const [rubberBandBox, setRubberBandBox] = useState<{
    startX: number;
    startY: number;
    currentX: number;
    currentY: number;
  } | null>(null);
  const marqueeDrag = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    clientX: number;
    clientY: number;
  } | null>(null);
  const stopMarquee = useCallback(() => {
    const drag = marqueeDrag.current;
    marqueeDrag.current = null;
    setRubberBandBox(null);
    const container = fileContainerRef.current;
    if (drag && container?.hasPointerCapture(drag.pointerId)) {
      container.releasePointerCapture(drag.pointerId);
    }
  }, []);

  useEffect(() => {
    stopMarquee();
    window.addEventListener('blur', stopMarquee);
    return () => window.removeEventListener('blur', stopMarquee);
  }, [currentPath, viewMode, stopMarquee]);

  // File Input Ref for Upload Button
  const uploadInputRef = useRef<HTMLInputElement>(null);

  // ----------------------------------------------------
  // Load Directory Entries
  // ----------------------------------------------------
  const loadFiles = useCallback(async (path: string = currentPath) => {
    const revision = ++listingSequence.current;
    setIsLoading(true); setLoadingMore(false);
    setErrorMessage(null);
    try {
      const res = await fetchFileList({
        path: path || undefined,
        sort_by: sortBy,
        sort_order: sortOrder,
        show_hidden: showHidden,
      });
      if (revision !== listingSequence.current) return;
      setEntries(res.entries || []); setTotalEntries(res.total); setHasMore(res.has_more);
      setSelectedPaths(new Set());
    } catch (err: unknown) {
      const e = err as Error;
      if (revision === listingSequence.current) { setEntries([]); setErrorMessage(e.message || 'Gagal memuat direktori berkas'); }
    } finally {
      if (revision === listingSequence.current) setIsLoading(false);
    }
  }, [currentPath, sortBy, sortOrder, showHidden, fetchFileList]);

  useEffect(() => {
    void loadFiles(currentPath);
    return () => { listingSequence.current++; };
  }, [loadFiles, currentPath]);
  const refreshAfterUpload = useRef(() => { void loadFiles(currentPath); });
  useEffect(() => {
    refreshAfterUpload.current = () => { void loadFiles(currentPath); };
  }, [loadFiles, currentPath]);

  // Load Trash Items
  const loadTrash = useCallback(async () => {
    setIsLoadingTrash(true);
    try {
      const res = await fetchTrashList();
      setTrashItems(res.items || []);
    } catch (err: unknown) {
      console.error('Failed to load trash list', err);
    } finally {
      setIsLoadingTrash(false);
    }
  }, [fetchTrashList]);

  // ----------------------------------------------------
  // Sorting and Client Filtering
  // ----------------------------------------------------
  const sortedAndFilteredEntries = React.useMemo(() => {
    let list = [...entries];

    if (searchFilter.trim()) {
      const q = searchFilter.toLowerCase();
      list = list.filter((e) => e.name.toLowerCase().includes(q));
    }

    list.sort((a, b) => {
      // Directories always come first
      if (a.is_dir && !b.is_dir) return -1;
      if (!a.is_dir && b.is_dir) return 1;

      let comparison = 0;
      if (sortBy === 'name') {
        comparison = a.name.localeCompare(b.name);
      } else if (sortBy === 'size') {
        comparison = (a.size_bytes || 0) - (b.size_bytes || 0);
      } else if (sortBy === 'modified') {
        comparison = (a.modified_at || 0) - (b.modified_at || 0);
      } else if (sortBy === 'extension') {
        const extA = a.name.split('.').pop() || '';
        const extB = b.name.split('.').pop() || '';
        comparison = extA.localeCompare(extB);
      }

      return sortOrder === 'asc' ? comparison : -comparison;
    });

    return list;
  }, [entries, searchFilter, sortBy, sortOrder]);

  // ----------------------------------------------------
  // Format Utilities
  // ----------------------------------------------------
  const formatFileSize = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
  };

  const formatDate = (epochSeconds: number) => {
    if (!epochSeconds) return '-';
    const date = new Date(epochSeconds * 1000);
    return date.toLocaleDateString('id-ID', {
      day: 'numeric',
      month: 'short',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  const getFileIcon = (entry: FsEntry) => {
    if (entry.is_dir) {
      return <Folder className="h-4 w-4 text-amber-500 shrink-0" />;
    }
    const ext = entry.name.split('.').pop()?.toLowerCase();
    switch (ext) {
      case 'php':
      case 'js':
      case 'ts':
      case 'jsx':
      case 'tsx':
      case 'py':
      case 'json':
      case 'html':
      case 'css':
      case 'sql':
      case 'sh':
        return <FileCode className="h-4 w-4 text-blue-500 shrink-0" />;
      case 'zip':
      case 'gz':
      case 'tar':
      case 'rar':
      case '7z':
        return <FileArchive className="h-4 w-4 text-purple-500 shrink-0" />;
      case 'png':
      case 'jpg':
      case 'jpeg':
      case 'svg':
      case 'webp':
      case 'gif':
        return <Image className="h-4 w-4 text-emerald-500 shrink-0" />;
      case 'mp4':
      case 'webm':
      case 'mkv':
        return <Film className="h-4 w-4 text-rose-500 shrink-0" />;
      case 'mp3':
      case 'wav':
      case 'ogg':
        return <Music className="h-4 w-4 text-indigo-500 shrink-0" />;
      case 'txt':
      case 'md':
      case 'log':
      case 'conf':
      case 'ini':
        return <FileText className="h-4 w-4 text-neutral-400 shrink-0" />;
      default:
        return <File className="h-4 w-4 text-neutral-400 shrink-0" />;
    }
  };

  // ----------------------------------------------------
  // Breadcrumb Segments
  // ----------------------------------------------------
  const breadcrumbSegments = React.useMemo(() => {
    if (!currentPath) return [];
    const parts = currentPath.split('/').filter(Boolean);
    return parts.map((part, index) => ({
      name: part,
      path: parts.slice(0, index + 1).join('/'),
    }));
  }, [currentPath]);

  const handleNavigatePath = (path: string) => {
    setSearchFilter(''); setSelectedPaths(new Set()); setLastSelectedIndex(null); setCurrentPath(path);
    if (window.matchMedia('(max-width: 760px)').matches) setIsTreeCollapsed(true);
  };

  const openEditorForFile = async (path: string, sizeBytes: number = 0) => {
    const fileName = path.split('/').pop() || '';
    try {
      const data = await fileApi.request<{content:string;size_bytes:number;modified_at?:number;etag?:string;read_only?:boolean;is_binary?:boolean}>(`/api/v1/files/content?path=${encodeURIComponent(path)}`);
      setActiveEditor({filePath:path,fileName,initialContent:data.content,sizeBytes:data.size_bytes ?? sizeBytes,mtime:data.modified_at,etag:data.etag,readOnly:data.read_only || data.is_binary,scope:tenant});
    } catch (err) { setErrorMessage(err instanceof Error ? err.message : 'Gagal membaca berkas'); }
  };

  const openPreviewForFile = (path: string, sizeBytes: number = 0) => {
    const fileName = path.split('/').pop() || '';
    setActivePreview({
      filePath: path,
      fileName,
      sizeBytes,
    });
  };

  const openPermissionForEntry = (entry: FsEntry) => {
    setActivePermission({
      name: entry.name,
      path: entry.path,
      isDir: entry.is_dir,
      modeOctal: (entry.mode & 0o777).toString(8).padStart(3, '0'),
      owner: entry.owner_user,
      group: entry.owner_group,
    });
  };

  const handleOpenItem = (entry: FsEntry) => {
    if (entry.is_dir) {
      handleNavigatePath(entry.path);
    } else {
      const ext = entry.name.split('.').pop()?.toLowerCase() || '';
      const isMedia = ['png', 'jpg', 'jpeg', 'svg', 'webp', 'gif', 'pdf', 'mp4', 'webm', 'mp3', 'wav'].includes(ext);
      if (isMedia) {
        openPreviewForFile(entry.path, entry.size_bytes);
      } else {
        openEditorForFile(entry.path, entry.size_bytes);
      }
    }
  };

  // ----------------------------------------------------
  // Selection Handlers (Single, Range, Multi)
  // ----------------------------------------------------
  const handleItemClick = (entry: FsEntry, index: number, e: React.MouseEvent) => {
    if (e.ctrlKey || e.metaKey) {
      // Toggle selection
      const next = new Set(selectedPaths);
      if (next.has(entry.path)) next.delete(entry.path);
      else next.add(entry.path);
      setSelectedPaths(next);
      setLastSelectedIndex(index);
    } else if (e.shiftKey && lastSelectedIndex !== null) {
      // Range selection
      const start = Math.min(lastSelectedIndex, index);
      const end = Math.max(lastSelectedIndex, index);
      const next = new Set(selectedPaths);
      for (let i = start; i <= end; i++) {
        if (sortedAndFilteredEntries[i]) {
          next.add(sortedAndFilteredEntries[i].path);
        }
      }
      setSelectedPaths(next);
    } else {
      // Single selection
      setSelectedPaths(new Set([entry.path]));
      setLastSelectedIndex(index);
    }
  };

  // ----------------------------------------------------
  // Keyboard Shortcuts (Ctrl+A, Delete, F2, Ctrl+C, Ctrl+V, Ctrl+F, Esc)
  // ----------------------------------------------------
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (document.querySelector('[role="dialog"][aria-modal="true"]') || activeEditor || e.defaultPrevented || (e.target instanceof HTMLElement && e.target.isContentEditable)) return;
      const isInput = ['INPUT', 'TEXTAREA', 'SELECT'].includes((e.target as HTMLElement)?.tagName);
      if (isInput) return;

      // Select All
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
        e.preventDefault();
        setSelectedPaths(new Set(sortedAndFilteredEntries.map((it) => it.path)));
      }
      // Delete / Backspace
      else if (e.key === 'Delete') {
        if (selectedPaths.size > 0) {
          e.preventDefault();
          setIsDeleteConfirmOpen(true);
        }
      }
      // F2 (Rename)
      else if (e.key === 'F2') {
        if (selectedPaths.size === 1) {
          e.preventDefault();
          const target = sortedAndFilteredEntries.find((it) => selectedPaths.has(it.path));
          if (target) {
            setRenameTarget(target);
            setRenameNewName(target.name);
            setIsRenameOpen(true);
          }
        }
      }
      // Copy (Ctrl+C)
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'c') {
        if (selectedPaths.size > 0) {
          e.preventDefault();
          setClipboard({ paths: Array.from(selectedPaths), operation: 'copy' });
        }
      }
      // Cut (Ctrl+X)
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'x') {
        if (selectedPaths.size > 0) {
          e.preventDefault();
          setClipboard({ paths: Array.from(selectedPaths), operation: 'cut' });
        }
      }
      // Paste (Ctrl+V)
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'v') {
        if (clipboard && clipboard.paths.length > 0) {
          e.preventDefault();
          handlePasteClipboard();
        }
      }
      // Search (Ctrl+F)
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        setIsSearchOpen(true);
      }
      // Escape (Clear selection or close context menu)
      else if (e.key === 'Escape') {
        setSelectedPaths(new Set());
        setContextMenu(null);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  });

  // ----------------------------------------------------
  // Clipboard Paste Operation
  // ----------------------------------------------------
  const handlePasteClipboard = async () => {
    if (!clipboard || clipboard.paths.length === 0) return;
    try {
      for (const src of clipboard.paths) {
        const fileName = src.split('/').pop() || '';
        const dest = currentPath ? `${currentPath}/${fileName}` : fileName;
        if (clipboard.operation === 'copy') {
          await copyFile(src, dest, false, false);
        } else {
          await moveFile(src, dest, false);
        }
      }
      if (clipboard.operation === 'cut') {
        setClipboard(null);
      }
      loadFiles(currentPath);
    } catch (err: unknown) {
      const e = err as Error;
      alert(`Gagal menempel berkas: ${e.message}`);
    }
  };

  // ----------------------------------------------------
  // Context Menu Actions
  // ----------------------------------------------------
  const handleContextMenu = (e: React.MouseEvent, entry?: FsEntry) => {
    e.preventDefault();
    e.stopPropagation();
    contextReturnFocus.current = e.currentTarget as HTMLElement;
    if (entry && !selectedPaths.has(entry.path)) {
      setSelectedPaths(new Set([entry.path]));
    }
    setContextMenu({
      x: Math.max(8, Math.min(e.clientX || e.currentTarget.getBoundingClientRect().left, window.innerWidth - 208)),
      y: Math.max(8, Math.min(e.clientY || e.currentTarget.getBoundingClientRect().bottom, window.innerHeight - 420)),
      targetEntry: entry,
    });
  };

  useEffect(() => {
    const handleClickOutside = () => setContextMenu(null);
    window.addEventListener('click', handleClickOutside);
    return () => window.removeEventListener('click', handleClickOutside);
  }, []);

  useEffect(() => {
    if (!contextMenu) return;
    const frame = window.requestAnimationFrame(() => contextMenuRef.current?.querySelector<HTMLButtonElement>('button')?.focus());
    return () => { window.cancelAnimationFrame(frame); if (contextReturnFocus.current?.isConnected) contextReturnFocus.current.focus({ preventScroll: true }); };
  }, [contextMenu]);
  useEffect(() => {
    if (!toolsOpen) return;
    const close = (event: PointerEvent) => { if (!toolsRef.current?.contains(event.target as Node)) setToolsOpen(false); };
    window.addEventListener('pointerdown', close);
    return () => window.removeEventListener('pointerdown', close);
  }, [toolsOpen]);

  // ----------------------------------------------------
  // File Mutation Actions (Create, Rename, Delete, etc.)
  // ----------------------------------------------------
  const handleCreateFolder = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newFolderName.trim()) return;
    const target = currentPath ? `${currentPath}/${newFolderName.trim()}` : newFolderName.trim();
    try {
      await createFolder(target, true);
      setIsNewFolderOpen(false);
      setNewFolderName('');
      loadFiles(currentPath);
    } catch (err: unknown) {
      const e = err as Error;
      alert(`Gagal membuat direktori: ${e.message}`);
    }
  };

  const handleCreateFile = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newFileName.trim()) return;
    const target = currentPath ? `${currentPath}/${newFileName.trim()}` : newFileName.trim();
    setIsNewFileOpen(false);
    setNewFileName('');
    openEditorForFile(target, 0);
  };

  const handleRename = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!renameTarget || !renameNewName.trim() || renameTarget.name === renameNewName.trim()) {
      setIsRenameOpen(false);
      return;
    }
    const parentDir = renameTarget.path.includes('/')
      ? renameTarget.path.substring(0, renameTarget.path.lastIndexOf('/'))
      : '';
    const newPath = parentDir ? `${parentDir}/${renameNewName.trim()}` : renameNewName.trim();
    try {
      await renameFile(renameTarget.path, newPath, false);
      setIsRenameOpen(false);
      setRenameTarget(null);
      loadFiles(currentPath);
    } catch (err: unknown) {
      const e = err as Error;
      alert(`Gagal mengganti nama: ${e.message}`);
    }
  };

  const handleDeleteSelected = async () => {
    try {
      for (const p of Array.from(selectedPaths)) {
        await moveToTrash(p);
      }
      setIsDeleteConfirmOpen(false);
      setSelectedPaths(new Set());
      loadFiles(currentPath);
    } catch (err: unknown) {
      const e = err as Error;
      alert(`Gagal memindahkan ke tempat sampah: ${e.message}`);
    }
  };

  // ----------------------------------------------------
  // Archive Compression & Extraction
  // ----------------------------------------------------
  const handleCompress = async (e: React.FormEvent) => {
    e.preventDefault();
    if (selectedPaths.size === 0) return;
    const targetName = compressName.trim() || 'archive.zip';
    const targetPath = currentPath ? `${currentPath}/${targetName}` : targetName;
    try {
      await compressArchive(Array.from(selectedPaths), targetPath, compressFormat);
      setIsCompressOpen(false);
      loadFiles(currentPath);
    } catch (err: unknown) {
      const e = err as Error;
      alert(`Gagal membuat arsip: ${e.message}`);
    }
  };

  const handleExtract = async (archivePath: string) => {
    try {
      await extractArchive(archivePath, currentPath);
      loadFiles(currentPath);
    } catch (err: unknown) {
      const e = err as Error;
      alert(`Gagal mengekstrak arsip: ${e.message}`);
    }
  };

  // ----------------------------------------------------
  // Chunked Upload Management
  // ----------------------------------------------------
  const handleFilesSelectedForUpload = (files: FileList | null) => {
    if (!files || files.length === 0) return;
    const newItems: UploadItem[] = Array.from(files).map((f) => ({
      id: `${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
      file: f,
      targetPath: currentPath ? `${currentPath}/${f.name}` : f.name,
      totalSize: f.size,
      uploadedSize: 0,
      status: 'pending',
      scope: tenant,
    }));

    setUploadQueue((prev) => [...prev, ...newItems]);
    setIsUploadTrayMinimized(false);
  };

  // Progress updates must not cancel the currently active upload.
  const activeUploadId = uploadQueue.find(item => item.scope === tenant && (item.status === 'pending' || item.status === 'uploading'))?.id;
  useEffect(() => {
    const pendingItem = uploadQueueRef.current.find(item => item.id === activeUploadId);
    if (!pendingItem) return;

    let isAborted = false;

    const processUpload = async () => {
      setUploadQueue((prev) =>
        prev.map((it) => (it.id === pendingItem.id ? { ...it, status: 'uploading' } : it))
      );

      try {
        const init = await initUpload(pendingItem.targetPath, pendingItem.totalSize);
        const chunkSize = init.chunk_size || 1024 * 1024 * 2; // 2MB default
        let offset = 0;

        while (offset < pendingItem.totalSize && !isAborted) {
          const chunkBlob = pendingItem.file.slice(offset, offset + chunkSize);
          const chunkRes = await uploadChunk(init.upload_id, offset, chunkBlob);
          if (isAborted) return;
          offset = chunkRes.current_offset;

          setUploadQueue((prev) =>
            prev.map((it) =>
              it.id === pendingItem.id ? { ...it, uploadedSize: offset } : it
            )
          );

          if (chunkRes.complete) break;
        }

        if (!isAborted) {
          setUploadQueue((prev) =>
            prev.map((it) =>
              it.id === pendingItem.id
                ? { ...it, status: 'completed', uploadedSize: it.totalSize }
                : it
            )
          );
          refreshAfterUpload.current();
        }
      } catch (err: unknown) {
        if (isAborted) return;
        const e = err as Error;
        setUploadQueue((prev) =>
          prev.map((it) =>
            it.id === pendingItem.id
              ? { ...it, status: 'error', errorMessage: e.message }
              : it
          )
        );
      }
    };

    processUpload();

    return () => {
      isAborted = true;
    };
  }, [activeUploadId, initUpload, uploadChunk]);

  // ----------------------------------------------------
  // Drag and Drop (Internal & Desktop File Drop)
  // ----------------------------------------------------
  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (!isDragOverDropzone) setIsDragOverDropzone(true);
  };

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOverDropzone(false);
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOverDropzone(false);

    // Desktop files drop
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
      handleFilesSelectedForUpload(e.dataTransfer.files);
      return;
    }

    // Internal move drop (from dragged items)
    const internalPaths = e.dataTransfer.getData('application/panel-files');
    if (internalPaths) {
      const pathsToMove: string[] = JSON.parse(internalPaths);
      for (const p of pathsToMove) {
        const fileName = p.split('/').pop() || '';
        const dest = currentPath ? `${currentPath}/${fileName}` : fileName;
        if (p !== dest) {
          await moveFile(p, dest, false).catch(() => {});
        }
      }
      loadFiles(currentPath);
    }
  };

  // ----------------------------------------------------
  // Rubber Band Marquee Drag Selection
  // ----------------------------------------------------
  const updateMarquee = (clientX: number, clientY: number) => {
    const drag = marqueeDrag.current;
    const container = fileContainerRef.current;
    if (!drag || !container) return;
    drag.clientX = clientX;
    drag.clientY = clientY;
    const rect = container.getBoundingClientRect();
    // The absolute overlay uses scroll-content coordinates, not viewport coordinates.
    const originX = rect.left + container.clientLeft - container.scrollLeft;
    const originY = rect.top + container.clientTop - container.scrollTop;
    setRubberBandBox({
      startX: drag.startX,
      startY: drag.startY,
      currentX: clientX - originX,
      currentY: clientY - originY,
    });

    // Convert the fixed content anchor back to the viewport for item hit testing.
    const left = Math.min(drag.startX + originX, clientX);
    const top = Math.min(drag.startY + originY, clientY);
    const right = Math.max(drag.startX + originX, clientX);
    const bottom = Math.max(drag.startY + originY, clientY);

    // Check intersecting items
    const nextSelected = new Set<string>();
    const itemElements = container.querySelectorAll('[data-file-item]');
    itemElements.forEach((el) => {
      const itemRect = el.getBoundingClientRect();
      const intersects =
        left < itemRect.right &&
        right > itemRect.left &&
        top < itemRect.bottom &&
        bottom > itemRect.top;
      if (intersects) {
        const path = el.getAttribute('data-path');
        if (path) nextSelected.add(path);
      }
    });

    setSelectedPaths(nextSelected);
  };

  const handlePointerDownForMarquee = (e: React.PointerEvent<HTMLDivElement>) => {
    // Leave touch scrolling, controls and native file dragging to the browser.
    if (e.button !== 0 || e.pointerType === 'touch' || marqueeDrag.current) return;
    const target = e.target as HTMLElement;
    if (target.closest('[data-file-item], button, input, select, textarea, a, thead, [role="button"]')) return;
    const container = e.currentTarget;
    const rect = container.getBoundingClientRect();
    const x = e.clientX - rect.left - container.clientLeft;
    const y = e.clientY - rect.top - container.clientTop;
    // Do not capture clicks on the scrollbars.
    if (x < 0 || y < 0 || x >= container.clientWidth || y >= container.clientHeight) return;
    e.preventDefault();
    container.setPointerCapture(e.pointerId);
    marqueeDrag.current = {
      pointerId: e.pointerId,
      startX: x + container.scrollLeft,
      startY: y + container.scrollTop,
      clientX: e.clientX,
      clientY: e.clientY,
    };
    updateMarquee(e.clientX, e.clientY);
  };

  const handlePointerMoveForMarquee = (e: React.PointerEvent<HTMLDivElement>) => {
    if (marqueeDrag.current?.pointerId !== e.pointerId) return;
    if ((e.buttons & 1) === 0) { stopMarquee(); return; }
    updateMarquee(e.clientX, e.clientY);
  };

  const handlePointerUpForMarquee = (e: React.PointerEvent<HTMLDivElement>) => {
    if (marqueeDrag.current?.pointerId !== e.pointerId) return;
    updateMarquee(e.clientX, e.clientY);
    stopMarquee();
  };

  const selectedEntry = selectedPaths.size === 1 ? entries.find(entry => selectedPaths.has(entry.path)) : undefined;
  const renameEntry = (entry: FsEntry) => { setRenameTarget(entry); setRenameNewName(entry.name); setIsRenameOpen(true); };
  const toggleEntry = (entry: FsEntry) => setSelectedPaths(previous => { const next = new Set(previous); if (next.has(entry.path)) next.delete(entry.path); else next.add(entry.path); return next; });
  const changeSort = (field: SortField) => { if (sortBy === field) setSortOrder(value => value === 'asc' ? 'desc' : 'asc'); else { setSortBy(field); setSortOrder('asc'); } };
  const openTerminal = () => { setToolsOpen(false); if (onNavigateTerminal) onNavigateTerminal(currentPath); else { setTerminalCwd(currentPath); setIsTerminalOpen(true); setTerminalOpenRequest(value => value + 1); } };
  const loadMoreFiles = async () => {
    if (loadingMore || isLoading || !hasMore) return;
    const revision = listingSequence.current; setLoadingMore(true);
    try {
      const result = await fetchFileList({ path: currentPath || undefined, sort_by: sortBy, sort_order: sortOrder, show_hidden: showHidden, offset: entries.length });
      if (revision === listingSequence.current) { setEntries(previous => [...previous, ...result.entries.filter(entry => !previous.some(existing => existing.path === entry.path))]); setTotalEntries(result.total); setHasMore(result.has_more); }
    } catch (cause) { if (revision === listingSequence.current) setErrorMessage(cause instanceof Error ? cause.message : 'Gagal memuat berkas berikutnya.'); }
    finally { if (revision === listingSequence.current) setLoadingMore(false); }
  };
  const inspector = selectedEntry ? <FileInspector entry={selectedEntry} english={english} icon={getFileIcon(selectedEntry)} onClose={() => { setShowDetails(false); setMobileDetails(false); }} onOpen={() => { setMobileDetails(false); handleOpenItem(selectedEntry); }} onPreview={() => { setMobileDetails(false); openPreviewForFile(selectedEntry.path, selectedEntry.size_bytes); }} onDownload={() => window.open(fileApi.url(`/api/v1/files/download?path=${encodeURIComponent(selectedEntry.path)}`), '_blank', 'noopener')} onRename={() => { setMobileDetails(false); renameEntry(selectedEntry); }} onPermissions={() => { setMobileDetails(false); openPermissionForEntry(selectedEntry); }} /> : null;

  // ----------------------------------------------------
  // RENDER UI
  // ----------------------------------------------------
  return (
    <div className="fb-manager" aria-label={title}>
      <input type="file" multiple ref={uploadInputRef} className="hidden" aria-label={text('Pilih berkas untuk diunggah', 'Choose files to upload')} onChange={event => handleFilesSelectedForUpload(event.target.files)} />
      <header className="fb-toolbar"><div className="fb-location"><button type="button" className="fb-icon-button" aria-label={text('Tampilkan navigasi folder', 'Toggle folder navigation')} aria-expanded={!isTreeCollapsed} onClick={() => setIsTreeCollapsed(value => !value)}><PanelLeft size={17} /></button><nav aria-label={text('Lokasi berkas', 'File location')}><button type="button" onClick={() => handleNavigatePath('')}><HardDrive size={15} />{text('Beranda', 'Home')}</button>{breadcrumbSegments.map((segment, index) => <React.Fragment key={segment.path}><ChevronRight size={13} /><button type="button" title={segment.name} aria-current={index === breadcrumbSegments.length - 1 ? 'location' : undefined} onClick={() => handleNavigatePath(segment.path)}>{segment.name}</button></React.Fragment>)}</nav></div>
        <div className="fb-toolbar-actions"><button type="button" className="fb-new-file" onClick={() => setIsNewFileOpen(true)}><FilePlus size={15} />{text('Berkas baru', 'New file')}</button><button type="button" onClick={() => setIsNewFolderOpen(true)}><FolderPlus size={15} />{text('Folder baru', 'New folder')}</button><button type="button" className="fb-primary" onClick={() => uploadInputRef.current?.click()}><Upload size={15} />{text('Unggah', 'Upload')}</button></div>
      </header>
      <div className="fb-controls"><div className="fb-search"><Search size={16} aria-hidden="true" /><input type="search" aria-label={text('Saring nama berkas', 'Filter file names')} placeholder={text('Cari dalam daftar berkas...', 'Search the file listing...')} value={searchFilter} onChange={event => setSearchFilter(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); setSearchFilter(''); } }} />{searchFilter && <button type="button" className="fb-icon-button" aria-label={text('Hapus pencarian', 'Clear search')} onClick={() => setSearchFilter('')}><X size={14} /></button>}</div>
        <div className="fb-view-controls"><button type="button" className="fb-deep-search" onClick={() => setIsSearchOpen(true)} title={text('Cari nama dan isi berkas (Ctrl+F)', 'Search file names and contents (Ctrl+F)')}><Search size={15} />{text('Pencarian lengkap', 'Advanced search')}</button><button type="button" className="fb-icon-button" disabled={isLoading} onClick={() => void loadFiles(currentPath)} aria-label={text('Segarkan berkas', 'Refresh files')}><RefreshCw size={16} className={isLoading ? 'animate-spin' : ''} /></button><div className="fb-view-switch" role="group" aria-label={text('Tampilan berkas', 'File view')}><button type="button" aria-label={text('Tampilan rinci', 'List view')} title="Tampilan Rinci" aria-pressed={viewMode === 'list'} onClick={() => setViewMode('list')}><LayoutList size={16} /></button><button type="button" aria-label={text('Tampilan grid', 'Grid view')} title="Tampilan Grid" aria-pressed={viewMode === 'grid'} onClick={() => setViewMode('grid')}><LayoutGrid size={16} /></button></div><div className="fb-tools" ref={toolsRef} onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setToolsOpen(false); terminalTrigger.current?.focus(); } }}><button type="button" className="fb-icon-button" aria-label={text('Opsi pengelola berkas', 'File manager options')} aria-expanded={toolsOpen} ref={terminalTrigger} onClick={() => setToolsOpen(value => !value)}><MoreHorizontal size={18} /></button>{toolsOpen && <div className="fb-tools-menu"><label><input type="checkbox" checked={showHidden} onChange={event => setShowHidden(event.target.checked)} /><EyeOff size={15} />{text('Tampilkan berkas tersembunyi', 'Show hidden files')}</label><button type="button" onClick={openTerminal} title="Buka terminal di folder ini"><Terminal size={15} />{text('Terminal di sini', 'Terminal here')}</button><button type="button" onClick={() => { setToolsOpen(false); setActiveSshPath(currentPath); setIsSshModalOpen(true); }}><Key size={15} />SSH / SFTP</button><button type="button" onClick={() => { setToolsOpen(false); void loadTrash(); setIsTrashOpen(true); }}><Trash2 size={15} />{text('Tempat sampah', 'Trash')}</button></div>}</div></div>
      </div>
      <div className="fb-selection-slot">{selectedPaths.size > 0 ? <div className="fb-selection-bar" role="region" aria-label={text('Tindakan berkas terpilih', 'Selected file actions')}><span><strong>{selectedPaths.size}</strong> {text('terpilih', 'selected')}</span><div>{selectedEntry && <><button type="button" onClick={() => handleOpenItem(selectedEntry)}><Folder size={15} />{text('Buka', 'Open')}</button><button type="button" className="fb-mobile-detail-button" onClick={() => setMobileDetails(true)}><Info size={15} />{text('Detail', 'Details')}</button><button type="button" className="fb-desktop-detail-button" aria-pressed={showDetails} onClick={() => setShowDetails(value => !value)}><Info size={15} />{text('Detail', 'Details')}</button><button type="button" onClick={() => renameEntry(selectedEntry)}><Edit2 size={15} />{text('Ganti nama', 'Rename')}</button></>}<button type="button" onClick={() => setClipboard({ paths: [...selectedPaths], operation: 'copy' })}><Copy size={15} />{text('Salin', 'Copy')}</button><button type="button" onClick={() => setClipboard({ paths: [...selectedPaths], operation: 'cut' })}><Scissors size={15} />{text('Potong', 'Cut')}</button><button type="button" onClick={() => setIsCompressOpen(true)}><Archive size={15} />{text('Arsipkan', 'Archive')}</button>{selectedEntry && <button type="button" onClick={() => openPermissionForEntry(selectedEntry)}><ShieldAlert size={15} />{text('Izin', 'Permissions')}</button>}<button type="button" className="fb-danger" onClick={() => setIsDeleteConfirmOpen(true)}><Trash2 size={15} />{text('Hapus', 'Delete')}</button></div><button type="button" className="fb-icon-button" aria-label={text('Batalkan pilihan', 'Clear selection')} onClick={() => setSelectedPaths(new Set())}><X size={15} /></button></div> : <div className="fb-list-caption"><h2>{currentPath.split('/').filter(Boolean).at(-1) || text('Berkas saya', 'My files')}</h2><span>{isLoading ? text('Memuat...', 'Loading...') : `${entries.length} item`}</span></div>}</div>
      {clipboard && <div className="fb-clipboard"><ClipboardPaste size={15} /><span>{clipboard.paths.length} {text('item siap', 'items ready')} {clipboard.operation === 'copy' ? text('disalin', 'to copy') : text('dipindahkan', 'to move')}</span><button type="button" onClick={() => void handlePasteClipboard()}>{text('Tempel di sini', 'Paste here')}</button><button type="button" className="fb-icon-button" aria-label={text('Kosongkan clipboard', 'Clear clipboard')} onClick={() => setClipboard(null)}><X size={14} /></button></div>}
      <div className="fb-body" data-navigation-open={!isTreeCollapsed}>
        {!isTreeCollapsed && <aside className="fb-sidebar" aria-label={text('Navigasi folder', 'Folder navigation')}><header><span>{text('Lokasi', 'Locations')}</span><button type="button" className="fb-icon-button" aria-label={text('Tutup navigasi folder', 'Close folder navigation')} onClick={() => setIsTreeCollapsed(true)}><PanelLeft size={15} /></button></header><button type="button" className="fb-home-link" aria-current={!currentPath ? 'location' : undefined} onClick={() => handleNavigatePath('')}><HardDrive size={16} />{text('Berkas saya', 'My files')}</button>{currentPath && <button type="button" className="fb-home-link" onClick={() => handleNavigatePath(currentPath.split('/').slice(0, -1).join('/'))}><ArrowUp size={15} />{text('Folder induk', 'Parent folder')}</button>}<div className="fb-sidebar-caption">{text('Folder di lokasi ini', 'Folders in this location')}</div><div className="fb-folder-links">{entries.filter(entry => entry.is_dir).map(entry => <button type="button" key={entry.path} onClick={() => handleNavigatePath(entry.path)}><Folder size={16} /><span>{entry.name}</span><ChevronRight size={12} /></button>)}{!isLoading && !entries.some(entry => entry.is_dir) && <p>{text('Tidak ada subfolder', 'No subfolders')}</p>}</div><div className="fb-sidebar-bottom"><button type="button" onClick={() => { void loadTrash(); setIsTrashOpen(true); }}><Trash2 size={16} />{text('Tempat sampah', 'Trash')}</button><p><ShieldAlert size={13} />{text('Ruang berkas akun hosting Anda', 'Your hosting account files')}</p></div></aside>}
        <div ref={fileContainerRef} className={`fb-viewport select-none overflow-auto relative ${isDragOverDropzone ? 'is-dragging' : ''}`} onPointerDown={handlePointerDownForMarquee} onPointerMove={handlePointerMoveForMarquee} onPointerUp={handlePointerUpForMarquee} onPointerCancel={stopMarquee} onLostPointerCapture={stopMarquee} onScrollCapture={() => { const drag = marqueeDrag.current; if (drag) updateMarquee(drag.clientX, drag.clientY); }} onContextMenu={event => handleContextMenu(event)} onDragOver={handleDragOver} onDragLeave={handleDragLeave} onDrop={handleDrop}>
          {isDragOverDropzone && <div className="fb-drop-overlay"><Upload size={32} /><strong>{text('Lepaskan berkas untuk mengunggah', 'Drop files to upload')}</strong><span>{currentPath || text('Beranda', 'Home')}</span></div>}
          {rubberBandBox && <div className="absolute border border-blue-500 pointer-events-none z-20 fb-marquee" style={{ left: Math.min(rubberBandBox.startX, rubberBandBox.currentX), top: Math.min(rubberBandBox.startY, rubberBandBox.currentY), width: Math.abs(rubberBandBox.currentX - rubberBandBox.startX), height: Math.abs(rubberBandBox.currentY - rubberBandBox.startY) }} />}
          {isLoading ? <div role="status" className="fb-loading"><span className="sr-only">{text('Memuat berkas...', 'Loading files...')}</span>{[0,1,2,3,4,5].map(value => <div key={value} />)}</div> : errorMessage ? <div className="fb-error" role="alert"><AlertTriangle size={20} /><h3>{text('Berkas belum dapat dimuat', 'Files could not be loaded')}</h3><p>{errorMessage}</p><button type="button" onClick={() => void loadFiles(currentPath)}>{text('Coba lagi', 'Retry')}</button></div> : sortedAndFilteredEntries.length ? <FileBrowserListing entries={sortedAndFilteredEntries} selected={selectedPaths} view={viewMode} sortBy={sortBy} sortOrder={sortOrder} onSort={changeSort} onSelect={handleItemClick} onToggle={toggleEntry} onSelectAll={checked => setSelectedPaths(new Set(checked ? sortedAndFilteredEntries.map(entry => entry.path) : []))} onOpen={handleOpenItem} onMenu={handleContextMenu} icon={getFileIcon} english={english} /> : <div className="fb-empty"><span><Folder size={30} /></span><h3>{searchFilter ? text('Tidak ada berkas yang cocok', 'No matching files') : text('Folder ini masih kosong', 'This folder is empty')}</h3><p>{searchFilter ? text('Coba nama lain atau gunakan pencarian lengkap untuk menelusuri subfolder dan isi berkas.', 'Try another name or use advanced search for subfolders and file contents.') : text('Unggah berkas atau buat folder untuk mulai menata project Anda.', 'Upload files or create a folder to organize your project.')}</p>{searchFilter ? <button type="button" onClick={() => setSearchFilter('')}>{text('Hapus pencarian', 'Clear search')}</button> : <button type="button" className="fb-primary" onClick={() => uploadInputRef.current?.click()}><Upload size={15} />{text('Unggah berkas', 'Upload files')}</button>}</div>}
          {!isLoading && !errorMessage && hasMore && <div className="fb-load-more"><p>{text('Pencarian cepat menyaring berkas yang sudah dimuat.', 'Quick search filters the loaded files.')}</p><button type="button" disabled={loadingMore} onClick={() => void loadMoreFiles()}>{loadingMore ? text('Memuat...', 'Loading...') : text('Muat lebih banyak', 'Load more')}</button></div>}
        </div>
        {selectedEntry && showDetails && !rubberBandBox && <aside className="fb-inspector-desktop">{inspector}</aside>}
      </div>
      <footer className="fb-status"><span>{isLoading ? text('Memuat...', 'Loading...') : `${entries.filter(entry => entry.is_dir).length} folder / ${entries.filter(entry => !entry.is_dir).length} ${text('berkas', 'files')}`}{hasMore && ` / ${totalEntries} ${text('item total', 'total items')}`}</span>{quotaInfo && <span className={quotaInfo.percentage > 90 ? 'fb-quota-high' : 'fb-quota'}>{text('Kuota:', 'Quota:')} {fileSize(quotaInfo.usedBytes)} / {fileSize(quotaInfo.limitBytes)} ({quotaInfo.percentage.toFixed(0)}%)</span>}<span className="fb-status-hint">{text('Klik dua kali atau tekan Enter untuk membuka', 'Double-click or press Enter to open')}</span></footer>
      {mobileDetails && selectedEntry && <ModalOverlay className="fb-modal fb-detail-overlay" variant="sheet" aria-label={text('Detail berkas', 'File details')} onDismiss={() => setMobileDetails(false)}><div className="dialog-panel fb-detail-sheet">{inspector}</div></ModalOverlay>}

      {/* 5. Upload Tray (Right Bottom Floating Widget) */}
      {uploadQueue.length > 0 && (
        <div className="fb-upload-tray absolute right-4 bottom-4 w-80 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-lg z-40 overflow-hidden text-xs">
          <div className="flex items-center justify-between px-3 py-2 bg-[var(--surface-hover)] border-b border-[var(--line)]">
            <div className="flex items-center gap-1.5 font-semibold">
              <Upload className="h-3.5 w-3.5 text-blue-500" />
              <span>Tray Unggah ({uploadQueue.length})</span>
            </div>
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => setIsUploadTrayMinimized(!isUploadTrayMinimized)}
                aria-label={isUploadTrayMinimized ? text('Perluas antrean unggah', 'Expand upload queue') : text('Minimalkan antrean unggah', 'Minimize upload queue')}
                className="p-1 rounded hover:bg-[var(--line)]"
              >
                <ChevronDown
                  className={`h-3.5 w-3.5 transition-transform ${
                    isUploadTrayMinimized ? 'rotate-180' : ''
                  }`}
                />
              </button>
              <button
                type="button"
                onClick={() => setUploadQueue([])}
                aria-label={text('Tutup antrean unggah', 'Close upload queue')}
                title={text('Hentikan antrean dan tutup', 'Stop the queue and close')}
                className="p-1 rounded hover:bg-[var(--line)]"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>

          {!isUploadTrayMinimized && (
            <div className="max-h-48 overflow-y-auto p-2 space-y-2">
              {uploadQueue.map((item) => {
                const percent =
                  item.totalSize > 0
                    ? Math.round((item.uploadedSize / item.totalSize) * 100)
                    : 0;
                return (
                  <div
                    key={item.id}
                    className="p-2 rounded border border-[var(--line)] bg-[var(--background)]"
                  >
                    <div className="flex items-center justify-between gap-2 mb-1">
                      <span className="font-medium truncate max-w-[160px]" title={item.file.name}>
                        {item.file.name}
                      </span>
                      <span className="font-mono text-[10px] text-[var(--muted-foreground)]">
                        {percent}%
                      </span>
                    </div>
                    <div role="progressbar" aria-label={item.file.name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} className="w-full h-1.5 rounded-full bg-[var(--surface-hover)] overflow-hidden">
                      <div
                        className={`h-full transition-all duration-150 ${
                          item.status === 'completed'
                            ? 'bg-emerald-500'
                            : item.status === 'error'
                            ? 'bg-red-500'
                            : 'bg-blue-500'
                        }`}
                        style={{ width: `${percent}%` }}
                      />
                    </div>
                    <p className="text-[10px] text-[var(--muted-foreground)] mt-1">{item.status === 'completed' ? text('Selesai', 'Completed') : item.status === 'uploading' ? text('Mengunggah', 'Uploading') : item.status === 'error' ? text('Gagal', 'Failed') : text('Menunggu', 'Queued')}</p>
                    {item.errorMessage && (
                      <p className="text-[10px] text-red-500 mt-1">{item.errorMessage}</p>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}

      {/* 6. Custom Context Menu */}
      {contextMenu && (
        <div
          ref={contextMenuRef} role="group" aria-label={text('Tindakan berkas', 'File actions')} onKeyDown={event => {
            const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
            const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
            const next = event.key === 'ArrowDown' ? (index + 1) % buttons.length : event.key === 'ArrowUp' ? (index - 1 + buttons.length) % buttons.length : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : -1;
            if (next >= 0) { event.preventDefault(); event.stopPropagation(); buttons[next]?.focus(); }
            if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setContextMenu(null); }
          }} className="fb-context-menu fixed z-50 w-48 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-lg py-1 text-xs animate-in fade-in zoom-in-95 duration-100"
          style={{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }}
          onClick={(e) => e.stopPropagation()}
        >
          {contextMenu.targetEntry ? (
            <>
              {contextMenu.targetEntry.is_dir ? (
                <button
                  type="button"
                  onClick={() => {
                    handleOpenItem(contextMenu.targetEntry!);
                    setContextMenu(null);
                  }}
                  className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
                >
                  <Folder className="h-3.5 w-3.5 text-amber-500" />
                  <span>Buka Direktori</span>
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => {
                      openEditorForFile(contextMenu.targetEntry!.path, contextMenu.targetEntry!.size_bytes);
                      setContextMenu(null);
                    }}
                    className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
                  >
                    <Edit2 className="h-3.5 w-3.5 text-blue-500" />
                    <span>Buka di Editor</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      openPreviewForFile(contextMenu.targetEntry!.path, contextMenu.targetEntry!.size_bytes);
                      setContextMenu(null);
                    }}
                    className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
                  >
                    <Eye className="h-3.5 w-3.5 text-emerald-500" />
                    <span>Pratinjau</span>
                  </button>
                </>
              )}

              <button
                type="button"
                onClick={() => {
                  window.open(fileApi.url(`/api/v1/files/download?path=${encodeURIComponent(contextMenu.targetEntry!.path)}`));
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <Download className="h-3.5 w-3.5" />
                <span>Unduh</span>
              </button>

              <button
                type="button"
                onClick={() => {
                  setRenameTarget(contextMenu.targetEntry!);
                  setRenameNewName(contextMenu.targetEntry!.name);
                  setIsRenameOpen(true);
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <Edit2 className="h-3.5 w-3.5 text-neutral-400" />
                <span>Ganti Nama (F2)</span>
              </button>

              <button
                type="button"
                onClick={() => {
                  openPermissionForEntry(contextMenu.targetEntry!);
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <ShieldAlert className="h-3.5 w-3.5 text-amber-500" />
                <span>Ubah Izin (chmod)</span>
              </button>

              {contextMenu.targetEntry.is_dir && (
                <button
                  type="button"
                  onClick={() => {
                    setActiveSshPath(contextMenu.targetEntry!.path);
                    setIsSshModalOpen(true);
                    setContextMenu(null);
                  }}
                  className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
                >
                  <Key className="h-3.5 w-3.5 text-blue-500" />
                  <span>Salin Perintah SSH</span>
                </button>
              )}

              {['zip', 'gz', 'tar'].includes(
                contextMenu.targetEntry.name.split('.').pop()?.toLowerCase() || ''
              ) && (
                <button
                  type="button"
                  onClick={() => {
                    handleExtract(contextMenu.targetEntry!.path);
                    setContextMenu(null);
                  }}
                  className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
                >
                  <Archive className="h-3.5 w-3.5 text-purple-500" />
                  <span>Ekstrak Arsip</span>
                </button>
              )}

              <div className="border-t border-[var(--line)] my-1" />

              <button
                type="button"
                onClick={() => {
                  setClipboard({ paths: [contextMenu.targetEntry!.path], operation: 'copy' });
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <Copy className="h-3.5 w-3.5" />
                <span>Salin (Ctrl+C)</span>
              </button>

              <button
                type="button"
                onClick={() => {
                  setClipboard({ paths: [contextMenu.targetEntry!.path], operation: 'cut' });
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <Scissors className="h-3.5 w-3.5" />
                <span>Potong (Ctrl+X)</span>
              </button>

              <div className="border-t border-[var(--line)] my-1" />

              <button
                type="button"
                onClick={() => {
                  setIsDeleteConfirmOpen(true);
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-red-500/10 text-red-500 text-left"
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>Pindahkan ke Sampah</span>
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                onClick={() => {
                  setIsNewFolderOpen(true);
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <FolderPlus className="h-3.5 w-3.5 text-amber-500" />
                <span>Folder Baru</span>
              </button>
              <button
                type="button"
                onClick={() => {
                  setIsNewFileOpen(true);
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <FilePlus className="h-3.5 w-3.5 text-blue-500" />
                <span>Berkas Baru</span>
              </button>
              <button
                type="button"
                onClick={() => {
                  uploadInputRef.current?.click();
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <Upload className="h-3.5 w-3.5 text-emerald-500" />
                <span>Unggah</span>
              </button>
              {clipboard && (
                <button
                  type="button"
                  onClick={() => {
                    handlePasteClipboard();
                    setContextMenu(null);
                  }}
                  className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
                >
                  <ClipboardPaste className="h-3.5 w-3.5 text-purple-500" />
                  <span>Tempel</span>
                </button>
              )}
              <div className="border-t border-[var(--line)] my-1" />
              <button
                type="button"
                onClick={() => {
                  setActiveSshPath(currentPath);
                  setIsSshModalOpen(true);
                  setContextMenu(null);
                }}
                className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-[var(--surface-hover)] text-left"
              >
                <Key className="h-3.5 w-3.5 text-blue-500" />
                <span>Salin Perintah SSH</span>
              </button>
            </>
          )}
        </div>
      )}

      {/* 7. Modal: Folder Baru */}
      {isNewFolderOpen && (
        <ModalOverlay aria-label="Buat Folder Baru"
          className="fb-modal fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsNewFolderOpen(false)}
        >
          <form
            onSubmit={handleCreateFolder}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Buat Folder Baru</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label htmlFor={`${formId}-newFolder`} className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Folder</label>
                <input id={`${formId}-newFolder`}
                  type="text"
                  autoFocus
                  value={newFolderName}
                  onChange={(e) => setNewFolderName(e.target.value)}
                  placeholder="nama-folder"
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
                />
              </div>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 dialog-footer">
              <button
                type="button"
                onClick={() => setIsNewFolderOpen(false)}
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)]"
              >
                Batal
              </button>
              <button
                type="submit"
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] bg-blue-600 hover:bg-blue-700 text-white font-medium"
              >
                Buat Folder
              </button>
            </div>
          </form>
        </ModalOverlay>
      )}

      {/* 8. Modal: Berkas Baru */}
      {isNewFileOpen && (
        <ModalOverlay aria-label="Buat Berkas Baru"
          className="fb-modal fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsNewFileOpen(false)}
        >
          <form
            onSubmit={handleCreateFile}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Buat Berkas Baru</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label htmlFor={`${formId}-newFile`} className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Berkas</label>
                <input id={`${formId}-newFile`}
                  type="text"
                  autoFocus
                  value={newFileName}
                  onChange={(e) => setNewFileName(e.target.value)}
                  placeholder="script.php"
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
                />
              </div>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 dialog-footer">
              <button
                type="button"
                onClick={() => setIsNewFileOpen(false)}
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)]"
              >
                Batal
              </button>
              <button
                type="submit"
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] bg-blue-600 hover:bg-blue-700 text-white font-medium"
              >
                Lanjut ke Editor
              </button>
            </div>
          </form>
        </ModalOverlay>
      )}

      {/* 9. Modal: Ganti Nama */}
      {isRenameOpen && (
        <ModalOverlay aria-label="Ganti Nama Berkas"
          className="fb-modal fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsRenameOpen(false)}
        >
          <form
            onSubmit={handleRename}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Ganti Nama Berkas</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label htmlFor={`${formId}-rename`} className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Baru</label>
                <input id={`${formId}-rename`}
                  type="text"
                  autoFocus
                  value={renameNewName}
                  onChange={(e) => setRenameNewName(e.target.value)}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
                />
              </div>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 dialog-footer">
              <button
                type="button"
                onClick={() => setIsRenameOpen(false)}
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)]"
              >
                Batal
              </button>
              <button
                type="submit"
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] bg-blue-600 hover:bg-blue-700 text-white font-medium"
              >
                Simpan Nama
              </button>
            </div>
          </form>
        </ModalOverlay>
      )}

      {/* 10. Modal: Arsipkan / Kompresi */}
      {isCompressOpen && (
        <ModalOverlay aria-label="Kompresi Menjadi Arsip"
          className="fb-modal fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsCompressOpen(false)}
        >
          <form
            onSubmit={handleCompress}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Kompresi Menjadi Arsip</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label htmlFor={`${formId}-archive`} className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Arsip</label>
                <input id={`${formId}-archive`}
                  type="text"
                  value={compressName}
                  onChange={(e) => setCompressName(e.target.value)}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none"
                />
              </div>
              <div>
                <label htmlFor={`${formId}-format`} className="text-xs text-[var(--muted-foreground)] block mb-1">Format</label>
                <select id={`${formId}-format`}
                  value={compressFormat}
                  onChange={(e) => setCompressFormat(e.target.value as 'zip' | 'tar_gz')}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none"
                >
                  <option value="zip">ZIP (.zip)</option>
                  <option value="tar_gz">TAR GZ (.tar.gz)</option>
                </select>
              </div>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 dialog-footer">
              <button
                type="button"
                onClick={() => setIsCompressOpen(false)}
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)]"
              >
                Batal
              </button>
              <button
                type="submit"
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] bg-purple-600 hover:bg-purple-700 text-white font-medium"
              >
                Mulai Kompresi
              </button>
            </div>
          </form>
        </ModalOverlay>
      )}

      {/* 11. Modal: Konfirmasi Hapus ke Sampah */}
      {isDeleteConfirmOpen && (
        <ModalOverlay aria-label="Pindahkan ke tempat sampah"
          className="fb-modal fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsDeleteConfirmOpen(false)}
        >
          <div className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel">
            <h3 className="text-sm font-semibold text-red-500 flex items-center gap-2 dialog-header">
              <Trash2 className="h-4 w-4" />
              <span>Pindahkan ke Tempat Sampah</span>
            </h3>
            <div className="dialog-body space-y-4">
              <p className="text-xs text-[var(--muted-foreground)]">
                Apakah Anda yakin ingin memindahkan {selectedPaths.size} berkas/folder terpilih ke tempat sampah?
                Berkas dapat dipulihkan kapan saja dari tempat sampah.
              </p>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 dialog-footer">
              <button
                type="button"
                onClick={() => setIsDeleteConfirmOpen(false)}
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)]"
              >
                Batal
              </button>
              <button
                type="button"
                onClick={handleDeleteSelected}
                className="px-3 py-1.5 text-xs rounded-[var(--radius-md)] bg-red-600 hover:bg-red-700 text-white font-medium"
              >
                Hapus ke Sampah
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* 12. Drawer: Tempat Sampah (.panel-trash) */}
      {isTrashOpen && (
        <ModalOverlay aria-label="Tempat Sampah"
          className="fb-modal fixed inset-0 z-50 flex justify-end bg-black/50 backdrop-blur-xs"
          onDismiss={() => setIsTrashOpen(false)}
          variant="sheet"
        >
          <div className="w-full max-w-md h-full bg-[var(--surface)] border-l border-[var(--line)] flex flex-col shadow-2xl animate-in slide-in-from-right duration-200 dialog-panel">
            <div className="flex items-center justify-between p-4 border-b border-[var(--line)] dialog-header">
              <div className="flex items-center gap-2 font-semibold text-sm">
                <Trash2 className="h-4 w-4 text-red-500" />
                <span>Tempat Sampah (.panel-trash)</span>
              </div>
              <button
                type="button"
                onClick={() => setIsTrashOpen(false)}
                className="p-1 rounded text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)]"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="p-3 border-b border-[var(--line)] flex items-center justify-between text-xs bg-[var(--background)]">
              <span className="text-[var(--muted-foreground)]">{trashItems.length} berkas di tempat sampah</span>
              {trashItems.length > 0 && (
                <button
                  type="button"
                  onClick={async () => {
                    if (confirm('Kosongkan seluruh tempat sampah secara permanen?')) {
                      await emptyTrash();
                      loadTrash();
                    }
                  }}
                  className="px-2 py-1 rounded text-red-500 hover:bg-red-500/10 font-medium"
                >
                  Kosongkan Sampah
                </button>
              )}
            </div>

            <div className="flex-1 overflow-y-auto p-3 space-y-2 text-xs dialog-body">
              {trashItems.map((item) => (
                <div
                  key={item.id}
                  className="p-2.5 rounded border border-[var(--line)] bg-[var(--background)] flex items-center justify-between gap-2"
                >
                  <div className="min-w-0">
                    <p className="font-medium truncate" title={item.original_path}>
                      {item.original_path}
                    </p>
                    <p className="text-[10px] text-[var(--muted-foreground)] font-mono">
                      {formatFileSize(item.size_bytes)} • Dihapus {formatDate(item.deleted_at)}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={async () => {
                      await restoreTrash(item.id);
                      loadTrash();
                      loadFiles(currentPath);
                    }}
                    className="flex items-center gap-1 px-2 py-1 rounded bg-blue-500/10 text-blue-500 hover:bg-blue-500/20 font-medium shrink-0"
                  >
                    <RotateCcw className="h-3 w-3" />
                    <span>Pulihkan</span>
                  </button>
                </div>
              ))}
              {trashItems.length === 0 && !isLoadingTrash && (
                <div className="text-center py-12 text-[var(--muted-foreground)]">Tempat sampah kosong.</div>
              )}
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* 13. Sub-Modals Integrations (P5-T6, P5-T7, P5-T8) */}
      {activeEditor && activeEditor.scope === tenant && (
        <FileEditorWorkspace
          key={tenant || 'self'}
          initialDocument={activeEditor}
          tenant={tenant}
          rootPath={baseRootPath}
          onClose={() => {
            setActiveEditor(null);
            void loadFiles(currentPath);
          }}
          onSaved={() => { void loadFiles(currentPath); }}
        />
      )}

      {activePreview && (
        <FilePreviewModal
          isOpen={true}
          filePath={activePreview.filePath}
          fileName={activePreview.fileName}
          apiBaseUrl={fileApi.url('/api/v1/files/download')}
          sizeBytes={activePreview.sizeBytes}
          onClose={() => setActivePreview(null)}
          onOpenEditor={() => {
            const p = activePreview.filePath;
            const sz = activePreview.sizeBytes;
            setActivePreview(null);
            openEditorForFile(p, sz);
          }}
        />
      )}

      {activePermission && (
        <FilePermissionModal
          isOpen={true}
          file={activePermission}
          onClose={() => setActivePermission(null)}
          onSave={async (payload) => {
            const endpoint = payload.isRecursive
              ? '/api/v1/files/chmod-recursive'
              : '/api/v1/files/chmod';
            await fileApi.request(endpoint, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                path: payload.path,
                mode: payload.mode,
                file_mode: payload.fileMode,
                dir_mode: payload.dirMode,
              }),
            });
            setActivePermission(null);
            loadFiles(currentPath);
          }}
        />
      )}

      {isSearchOpen && (
        <FileSearchModal
          isOpen={true}
          currentPath={currentPath}
          onExecuteSearch={async params => {
            const report = await fileApi.request<{items:{path:string;is_dir:boolean;size_bytes:number;modified_at?:number;matches:{line_number:number;line_text:string}[]}[]; total_scanned:number;limit_reached:boolean;timed_out:boolean;duration_ms:number}>('/api/v1/files/search', {method:'POST',body:JSON.stringify({path:params.path,query:params.query,search_content:params.searchContent,case_sensitive:params.caseSensitive,extension:params.extension})});
            return {items:report.items.map(item=>({path:item.path,isDir:item.is_dir,sizeBytes:item.size_bytes,modifiedAt:item.modified_at,matches:item.matches.map(m=>({lineNumber:m.line_number,lineText:m.line_text}))})),totalScanned:report.total_scanned,limitReached:report.limit_reached,timedOut:report.timed_out,durationMs:report.duration_ms};
          }}
          onClose={() => setIsSearchOpen(false)}
          onOpenFileEditor={(p: string) => {
            setIsSearchOpen(false);
            openEditorForFile(p, 0);
          }}
          onOpenFilePreview={(p: string) => {
            setIsSearchOpen(false);
            openPreviewForFile(p, 0);
          }}
        />
      )}

      <TerminalDrawer
        isOpen={isTerminalOpen}
        openRequest={terminalOpenRequest}
        onClose={() => { setIsTerminalOpen(false); terminalTrigger.current?.focus(); }}
        initialCwd={terminalCwd}
        tenant={tenant}
      />

      <SshConnectModal
        isOpen={isSshModalOpen}
        onClose={() => setIsSshModalOpen(false)}
        path={activeSshPath || currentPath}
        tenantUsername={tenant}
        siteId={siteId}
        onOpenTerminal={() => {
          if (onNavigateTerminal) onNavigateTerminal(activeSshPath || currentPath);
          else { setTerminalCwd(activeSshPath || currentPath); setIsTerminalOpen(true); setTerminalOpenRequest(previous => previous + 1); }
        }}
      />
    </div>
  );
};
