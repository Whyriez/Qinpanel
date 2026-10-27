import { ModalOverlay } from '../ui/ModalOverlay';
import React, {
  useState,
  useEffect,
  useCallback,
  useRef,
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
  Sparkles,
  Key,
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

interface TreeNode {
  name: string;
  path: string;
  isExpanded: boolean;
  children: TreeNode[];
  isLoading: boolean;
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
  const [showHidden] = useState(false);

  // Selection state
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [lastSelectedIndex, setLastSelectedIndex] = useState<number | null>(null);

  // Tree directory state (Left Sidebar)
  const [isTreeCollapsed, setIsTreeCollapsed] = useState(() => typeof window !== 'undefined' && window.matchMedia('(max-width: 640px)').matches);
  const [treeRoot] = useState<TreeNode[]>([
    { name: 'sites', path: 'sites', isExpanded: true, children: [], isLoading: false },
    { name: 'tmp', path: 'tmp', isExpanded: false, children: [], isLoading: false },
  ]);

  // Internal File Clipboard (Copy / Cut)
  const [clipboard, setClipboard] = useState<{
    paths: string[];
    operation: 'copy' | 'cut';
  } | null>(null);

  // Upload Tray State
  const [uploadQueue, setUploadQueue] = useState<UploadItem[]>([]);
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
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const res = await fetchFileList({
        path: path || undefined,
        sort_by: sortBy,
        sort_order: sortOrder,
        show_hidden: showHidden,
      });
      setEntries(res.entries || []);
      setSelectedPaths(new Set());
    } catch (err: unknown) {
      const e = err as Error;
      setErrorMessage(e.message || 'Gagal memuat direktori berkas');
    } finally {
      setIsLoading(false);
    }
  }, [currentPath, sortBy, sortOrder, showHidden, fetchFileList]);

  useEffect(() => {
    loadFiles(currentPath);
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

  const formatPermissions = (mode: number) => {
    return (mode & 0o777).toString(8).padStart(3, '0');
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
    setCurrentPath(path);
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

  const handleSelectAll = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.checked) {
      setSelectedPaths(new Set(sortedAndFilteredEntries.map((e) => e.path)));
    } else {
      setSelectedPaths(new Set());
    }
  };

  // ----------------------------------------------------
  // Keyboard Shortcuts (Ctrl+A, Delete, F2, Ctrl+C, Ctrl+V, Ctrl+F, Esc)
  // ----------------------------------------------------
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (activeEditor || e.defaultPrevented || (e.target instanceof HTMLElement && e.target.isContentEditable)) return;
      const isInput = ['INPUT', 'TEXTAREA'].includes((e.target as HTMLElement)?.tagName);
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
    if (entry && !selectedPaths.has(entry.path)) {
      setSelectedPaths(new Set([entry.path]));
    }
    setContextMenu({
      x: e.clientX,
      y: e.clientY,
      targetEntry: entry,
    });
  };

  useEffect(() => {
    const handleClickOutside = () => setContextMenu(null);
    window.addEventListener('click', handleClickOutside);
    return () => window.removeEventListener('click', handleClickOutside);
  }, []);

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
    }));

    setUploadQueue((prev) => [...prev, ...newItems]);
    setIsUploadTrayMinimized(false);
  };

  // Process upload items sequentially
  useEffect(() => {
    const pendingItem = uploadQueue.find((it) => it.status === 'pending');
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
          loadFiles(currentPath);
        }
      } catch (err: unknown) {
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
  }, [uploadQueue, currentPath, loadFiles]);

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

  // ----------------------------------------------------
  // RENDER UI
  // ----------------------------------------------------
  return (
    <div
      className="flex flex-col h-[calc(100vh-4rem)] border border-[var(--line)] rounded-[var(--radius-lg)] bg-[var(--surface)] text-[var(--foreground)] overflow-hidden shadow-xs"
    >
      {/* Hidden File Upload Input */}
      <input
        type="file"
        multiple
        ref={uploadInputRef}
        className="hidden"
        onChange={(e) => handleFilesSelectedForUpload(e.target.files)}
      />

      {/* 1. Header Toolbar & Quota Info */}
      <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3 border-b border-[var(--line)] bg-[var(--surface)]">
        {/* Left: Breadcrumbs & Path */}
        <div className="flex items-center gap-1.5 min-w-0 flex-1 overflow-x-auto text-xs py-1">
          {title && (
            <span className="font-semibold text-[var(--foreground)] mr-1 hidden sm:inline truncate max-w-[160px]">
              {title}
            </span>
          )}
          <button
            type="button"
            onClick={() => handleNavigatePath('')}
            className={`flex items-center gap-1 px-2 py-1 rounded-[var(--radius-sm)] font-medium transition-colors ${
              currentPath === ''
                ? 'bg-[var(--surface-hover)] text-blue-500 font-semibold'
                : 'text-[var(--muted-foreground)] hover:text-[var(--foreground)] hover:bg-[var(--surface-hover)]'
            }`}
          >
            <HardDrive className="h-3.5 w-3.5" />
            <span>root</span>
          </button>

          {breadcrumbSegments.map((seg, idx) => (
            <React.Fragment key={seg.path}>
              <ChevronRight className="h-3 w-3 text-[var(--line)] shrink-0" />
              <button
                type="button"
                onClick={() => handleNavigatePath(seg.path)}
                className={`px-2 py-1 rounded-[var(--radius-sm)] truncate max-w-[140px] transition-colors ${
                  idx === breadcrumbSegments.length - 1
                    ? 'bg-[var(--surface-hover)] text-blue-500 font-semibold'
                    : 'text-[var(--muted-foreground)] hover:text-[var(--foreground)] hover:bg-[var(--surface-hover)]'
                }`}
                title={seg.name}
              >
                {seg.name}
              </button>
            </React.Fragment>
          ))}
        </div>

        {/* Right: Quota Meter & Terminal Button */}
        <div className="flex items-center gap-3 shrink-0 text-xs">
          {quotaInfo && (
            <div className="flex items-center gap-2 px-2.5 py-1 rounded-[var(--radius-sm)] bg-[var(--background)] border border-[var(--line)]">
              <span className="text-[var(--muted-foreground)] text-[11px]">Kuota:</span>
              <span className="font-mono text-[11px] font-medium">
                {formatFileSize(quotaInfo.usedBytes)} / {formatFileSize(quotaInfo.limitBytes)}
              </span>
              <div className="w-16 h-1.5 rounded-full bg-[var(--surface-hover)] overflow-hidden">
                <div
                  className={`h-full rounded-full transition-all duration-300 ${
                    quotaInfo.percentage > 90
                      ? 'bg-red-500'
                      : quotaInfo.percentage > 75
                      ? 'bg-amber-500'
                      : 'bg-emerald-500'
                  }`}
                  style={{ width: `${Math.min(quotaInfo.percentage, 100)}%` }}
                />
              </div>
            </div>
          )}

          {/* Tombol Terminal di sini */}
          <div className="relative group">
            <button
              type="button"
              onClick={() => {
                if (onNavigateTerminal) {
                  onNavigateTerminal(currentPath);
                } else {
                  setTerminalCwd(currentPath);
                  setIsTerminalOpen(true);
                  setTerminalOpenRequest(previous => previous + 1);
                }
              }}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface-hover)] hover:bg-[var(--surface-active)] text-[var(--foreground)] transition-colors text-xs font-medium cursor-pointer"
              title="Buka terminal di folder ini"
              ref={terminalTrigger}
            >
              <Terminal className="h-3.5 w-3.5 text-emerald-500" />
              <span>Terminal di sini</span>
            </button>
          </div>

          {/* Tombol Salin SSH */}
          <div className="relative group">
            <button
              type="button"
              onClick={() => {
                setActiveSshPath(currentPath);
                setIsSshModalOpen(true);
              }}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface-hover)] hover:bg-[var(--surface-active)] text-[var(--foreground)] transition-colors text-xs font-medium cursor-pointer"
              title="Salin perintah koneksi SSH & SFTP"
            >
              <Key className="h-3.5 w-3.5 text-blue-500" />
              <span>Salin SSH</span>
            </button>
          </div>
        </div>
      </div>

      {/* 2. Action Ribbon */}
      <div className="flex flex-wrap items-center justify-between gap-2 px-4 py-2 border-b border-[var(--line)] bg-[var(--surface)]/70 text-xs">
        {/* Left Action Buttons */}
        <div className="flex items-center gap-1.5 flex-wrap">
          <button
            type="button"
            onClick={() => uploadInputRef.current?.click()}
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] bg-blue-600 hover:bg-blue-700 text-white font-medium shadow-xs transition-colors"
          >
            <Upload className="h-3.5 w-3.5" />
            <span>Unggah</span>
          </button>

          <button
            type="button"
            onClick={() => setIsNewFolderOpen(true)}
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] transition-colors font-medium"
          >
            <FolderPlus className="h-3.5 w-3.5 text-amber-500" />
            <span>Folder Baru</span>
          </button>

          <button
            type="button"
            onClick={() => setIsNewFileOpen(true)}
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] transition-colors font-medium"
          >
            <FilePlus className="h-3.5 w-3.5 text-blue-500" />
            <span>Berkas Baru</span>
          </button>

            <button
              type="button"
              disabled={selectedPaths.size === 0}
              onClick={() => setIsCompressOpen(true)}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] transition-colors font-medium disabled:opacity-50 disabled:cursor-default"
            >
              <Archive className="h-3.5 w-3.5 text-purple-500" />
              <span>Arsipkan</span>
            </button>

          {clipboard && (
            <button
              type="button"
              onClick={handlePasteClipboard}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] bg-emerald-600/10 text-emerald-600 border border-emerald-500/20 hover:bg-emerald-600/20 transition-colors font-medium"
            >
              <ClipboardPaste className="h-3.5 w-3.5" />
              <span>Tempel ({clipboard.paths.length})</span>
            </button>
          )}

          <button
            type="button"
            onClick={() => {
              loadTrash();
              setIsTrashOpen(true);
            }}
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] transition-colors text-[var(--muted-foreground)] hover:text-[var(--foreground)]"
            title="Buka Tempat Sampah"
          >
            <Trash2 className="h-3.5 w-3.5" />
            <span>Tempat Sampah</span>
          </button>
        </div>

        {/* Right Search & Controls */}
        <div className="flex items-center gap-2">
          {/* Quick Client Filter */}
          <div className="relative">
            <Search className="h-3.5 w-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--muted-foreground)]" />
            <input
              type="text"
              placeholder="Saring..."
              value={searchFilter}
              onChange={(e) => setSearchFilter(e.target.value)}
              className="pl-8 pr-2 py-1 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] w-28 focus:w-40 transition-all focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>

          {/* Deep Content Search Modal Trigger */}
          <button
            type="button"
            onClick={() => setIsSearchOpen(true)}
            className="p-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] text-[var(--muted-foreground)] hover:text-[var(--foreground)] transition-colors"
            title="Pencarian Mendalam (Ctrl+F)"
          >
            <Sparkles className="h-3.5 w-3.5 text-blue-500" />
          </button>

          {/* Refresh Button */}
          <button
            type="button"
            onClick={() => loadFiles(currentPath)}
            className="p-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] text-[var(--muted-foreground)] hover:text-[var(--foreground)] transition-colors"
            title="Segarkan Berkas"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          </button>

          {/* List / Grid Toggle */}
          <div className="flex items-center rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] p-0.5">
            <button
              type="button"
              onClick={() => setViewMode('list')}
              className={`p-1 rounded-[var(--radius-sm)] transition-colors ${
                viewMode === 'list'
                  ? 'bg-[var(--surface)] text-[var(--foreground)] shadow-xs'
                  : 'text-[var(--muted-foreground)] hover:text-[var(--foreground)]'
              }`}
              title="Tampilan Rinci"
            >
              <LayoutList className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              onClick={() => setViewMode('grid')}
              className={`p-1 rounded-[var(--radius-sm)] transition-colors ${
                viewMode === 'grid'
                  ? 'bg-[var(--surface)] text-[var(--foreground)] shadow-xs'
                  : 'text-[var(--muted-foreground)] hover:text-[var(--foreground)]'
              }`}
              title="Tampilan Grid"
            >
              <LayoutGrid className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>

      {/* 3. Main Body: Split View (Directory Tree + Files View) */}
      <div className="flex flex-1 min-h-0 relative overflow-hidden">
        {/* Left: Collapsible Directory Tree */}
        <div
          className={`border-r border-[var(--line)] bg-[var(--surface)]/50 flex flex-col transition-all duration-200 ${
            isTreeCollapsed ? 'w-0 opacity-0 overflow-hidden border-none' : 'w-56 shrink-0'
          }`}
        >
          <div className="p-2 border-b border-[var(--line)] text-[11px] font-semibold text-[var(--muted-foreground)] uppercase tracking-wider flex items-center justify-between">
            <span>Pohon Folder</span>
            <button
              type="button"
              onClick={() => setIsTreeCollapsed(true)}
              className="text-[var(--muted-foreground)] hover:text-[var(--foreground)] p-0.5"
            >
              <ChevronRight className="h-3 w-3 rotate-180" />
            </button>
          </div>

          <div className="flex-1 overflow-y-auto p-2 space-y-1 text-xs">
            <button
              type="button"
              onClick={() => handleNavigatePath('')}
              className={`w-full flex items-center gap-2 px-2 py-1.5 rounded-[var(--radius-sm)] text-left transition-colors ${
                currentPath === ''
                  ? 'bg-blue-500/10 text-blue-500 font-medium'
                  : 'text-[var(--foreground)] hover:bg-[var(--surface-hover)]'
              }`}
            >
              <HardDrive className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">root</span>
            </button>

            {treeRoot.map((node) => (
              <div key={node.path} className="space-y-0.5">
                <button
                  type="button"
                  onClick={() => handleNavigatePath(node.path)}
                  className={`w-full flex items-center gap-1.5 px-2 py-1 rounded-[var(--radius-sm)] text-left transition-colors ${
                    currentPath === node.path || currentPath.startsWith(`${node.path}/`)
                      ? 'bg-blue-500/10 text-blue-500 font-medium'
                      : 'text-[var(--foreground)] hover:bg-[var(--surface-hover)]'
                  }`}
                >
                  <Folder className="h-3.5 w-3.5 text-amber-500 shrink-0" />
                  <span className="truncate">{node.name}</span>
                </button>
              </div>
            ))}
          </div>
        </div>

        {/* Tree Expand Toggle when collapsed */}
        {isTreeCollapsed && (
          <button
            type="button"
            onClick={() => setIsTreeCollapsed(false)}
            className="absolute left-0 top-2 z-10 p-1 rounded-r border border-l-0 border-[var(--line)] bg-[var(--surface)] text-[var(--muted-foreground)] hover:text-[var(--foreground)] shadow-xs"
            title="Buka Pohon Folder"
          >
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        )}

        {/* Right: Files Container (with Dropzone & Rubber Band) */}
        <div
          ref={fileContainerRef}
          onPointerDown={handlePointerDownForMarquee}
          onPointerMove={handlePointerMoveForMarquee}
          onPointerUp={handlePointerUpForMarquee}
          onPointerCancel={stopMarquee}
          onLostPointerCapture={stopMarquee}
          onScrollCapture={() => {
            const drag = marqueeDrag.current;
            if (drag) updateMarquee(drag.clientX, drag.clientY);
          }}
          onContextMenu={(e) => handleContextMenu(e)}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          onDrop={handleDrop}
          className={`flex-1 min-w-0 overflow-auto bg-[var(--background)] relative p-3 select-none ${
            isDragOverDropzone ? 'ring-2 ring-blue-500 ring-inset bg-blue-500/5' : ''
          }`}
        >
          {/* Dropzone Overlay Visual */}
          {isDragOverDropzone && (
            <div className="absolute inset-0 flex flex-col items-center justify-center bg-blue-500/10 backdrop-blur-xs z-30 pointer-events-none">
              <Upload className="h-10 w-10 text-blue-500 animate-bounce mb-2" />
              <p className="text-sm font-semibold text-blue-500">
                Lepaskan berkas di sini untuk mengunggah ke folder aktif
              </p>
            </div>
          )}

          {/* Rubber Band Selection Box */}
          {rubberBandBox && (
            <div
              className="absolute border border-blue-500 bg-blue-500/15 pointer-events-none z-20 rounded-xs"
              style={{
                left: `${Math.min(rubberBandBox.startX, rubberBandBox.currentX)}px`,
                top: `${Math.min(rubberBandBox.startY, rubberBandBox.currentY)}px`,
                width: `${Math.abs(rubberBandBox.currentX - rubberBandBox.startX)}px`,
                height: `${Math.abs(rubberBandBox.currentY - rubberBandBox.startY)}px`,
              }}
            />
          )}

          {/* Loading Skeleton */}
          {isLoading && (
            <div className="space-y-2 py-4">
              {[1, 2, 3, 4, 5, 6].map((i) => (
                <div
                  key={i}
                  className="h-9 w-full rounded-[var(--radius-sm)] bg-[var(--surface-hover)] animate-pulse"
                />
              ))}
            </div>
          )}

          {/* Error State */}
          {!isLoading && errorMessage && (
            <div className="p-4 rounded-[var(--radius-md)] border border-red-500/30 bg-red-500/10 text-red-500 flex items-center justify-between text-xs">
              <div className="flex items-center gap-2">
                <AlertTriangle className="h-4 w-4 shrink-0" />
                <span>{errorMessage}</span>
              </div>
              <button
                type="button"
                onClick={() => loadFiles(currentPath)}
                className="px-2 py-1 rounded bg-red-600 text-white font-medium hover:bg-red-700"
              >
                Coba Lagi
              </button>
            </div>
          )}

          {/* Empty State */}
          {!isLoading && !errorMessage && sortedAndFilteredEntries.length === 0 && (
            <div className="flex flex-col items-center justify-center py-16 text-center text-[var(--muted-foreground)]">
              <div className="flex h-14 w-14 items-center justify-center rounded-full bg-[var(--surface-hover)] text-[var(--muted-foreground)] mb-3">
                <Folder className="h-7 w-7 stroke-1" />
              </div>
              <h4 className="text-sm font-semibold text-[var(--foreground)] mb-1">
                Direktori ini masih kosong
              </h4>
              <p className="text-xs max-w-sm mb-4">
                Unggah berkas dari perangkat Anda atau buat folder baru untuk memulai pengelolaan web.
              </p>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => uploadInputRef.current?.click()}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-[var(--radius-md)] bg-blue-600 hover:bg-blue-700 text-white font-medium text-xs shadow-xs"
                >
                  <Upload className="h-3.5 w-3.5" />
                  <span>Unggah Berkas</span>
                </button>
                <button
                  type="button"
                  onClick={() => setIsNewFolderOpen(true)}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-hover)] font-medium text-xs"
                >
                  <FolderPlus className="h-3.5 w-3.5" />
                  <span>Folder Baru</span>
                </button>
              </div>
            </div>
          )}

          {/* LIST VIEW */}
          {!isLoading && !errorMessage && sortedAndFilteredEntries.length > 0 && viewMode === 'list' && (
            <div className="overflow-x-auto">
              <table className="w-full text-left text-xs border-collapse">
                <thead>
                  <tr className="border-b border-[var(--line)] text-[var(--muted-foreground)] font-semibold select-none">
                    <th className="py-2 px-2.5 w-8">
                      <input
                        type="checkbox"
                        checked={
                          selectedPaths.size > 0 &&
                          selectedPaths.size === sortedAndFilteredEntries.length
                        }
                        onChange={handleSelectAll}
                        className="rounded border-[var(--line)] text-blue-600"
                      />
                    </th>
                    <th
                      className="py-2 px-2 cursor-pointer hover:text-[var(--foreground)]"
                      onClick={() => {
                        if (sortBy === 'name') setSortOrder((o) => (o === 'asc' ? 'desc' : 'asc'));
                        else {
                          setSortBy('name');
                          setSortOrder('asc');
                        }
                      }}
                    >
                      Nama
                    </th>
                    <th
                      className="py-2 px-2.5 w-24 cursor-pointer hover:text-[var(--foreground)]"
                      onClick={() => {
                        if (sortBy === 'size') setSortOrder((o) => (o === 'asc' ? 'desc' : 'asc'));
                        else {
                          setSortBy('size');
                          setSortOrder('asc');
                        }
                      }}
                    >
                      Ukuran
                    </th>
                    <th
                      className="py-2 px-2.5 w-36 cursor-pointer hover:text-[var(--foreground)]"
                      onClick={() => {
                        if (sortBy === 'modified')
                          setSortOrder((o) => (o === 'asc' ? 'desc' : 'asc'));
                        else {
                          setSortBy('modified');
                          setSortOrder('asc');
                        }
                      }}
                    >
                      Diubah
                    </th>
                    <th className="py-2 px-2.5 w-16">Izin</th>
                    <th className="py-2 px-2.5 w-20">Pemilik</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)]/50">
                  {sortedAndFilteredEntries.map((entry, idx) => {
                    const isSelected = selectedPaths.has(entry.path);
                    return (
                      <tr
                        key={entry.path}
                        data-file-item="true"
                        data-path={entry.path}
                        draggable
                        onDragStart={(e) => {
                          const paths = selectedPaths.has(entry.path)
                            ? Array.from(selectedPaths)
                            : [entry.path];
                          e.dataTransfer.setData('application/panel-files', JSON.stringify(paths));
                        }}
                        onClick={(e) => handleItemClick(entry, idx, e)}
                        onDoubleClick={() => handleOpenItem(entry)}
                        onContextMenu={(e) => handleContextMenu(e, entry)}
                        className={`transition-colors cursor-pointer group ${
                          isSelected
                            ? 'bg-blue-500/10 font-medium'
                            : 'hover:bg-[var(--surface-hover)]'
                        }`}
                      >
                        <td className="py-2 px-2.5">
                          <input
                            type="checkbox"
                            checked={isSelected}
                            onChange={() => {}}
                            className="rounded border-[var(--line)] text-blue-600 pointer-events-none"
                          />
                        </td>
                        <td className="py-2 px-2 font-medium flex items-center gap-2 truncate">
                          {getFileIcon(entry)}
                          <span className="truncate">{entry.name}</span>
                        </td>
                        <td className="py-2 px-2.5 text-[var(--muted-foreground)] font-mono text-[11px]">
                          {entry.is_dir ? '—' : formatFileSize(entry.size_bytes)}
                        </td>
                        <td className="py-2 px-2.5 text-[var(--muted-foreground)]">
                          {formatDate(entry.modified_at)}
                        </td>
                        <td className="py-2 px-2.5 font-mono text-[11px] text-[var(--muted-foreground)]">
                          {formatPermissions(entry.mode)}
                        </td>
                        <td className="py-2 px-2.5 text-[var(--muted-foreground)] truncate">
                          {entry.owner_user}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}

          {/* GRID VIEW */}
          {!isLoading && !errorMessage && sortedAndFilteredEntries.length > 0 && viewMode === 'grid' && (
            <div className="grid grid-cols-2 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8 gap-3">
              {sortedAndFilteredEntries.map((entry, idx) => {
                const isSelected = selectedPaths.has(entry.path);
                return (
                  <div
                    key={entry.path}
                    data-file-item="true"
                    data-path={entry.path}
                    draggable
                    onDragStart={(e) => {
                      const paths = selectedPaths.has(entry.path)
                        ? Array.from(selectedPaths)
                        : [entry.path];
                      e.dataTransfer.setData('application/panel-files', JSON.stringify(paths));
                    }}
                    onClick={(e) => handleItemClick(entry, idx, e)}
                    onDoubleClick={() => handleOpenItem(entry)}
                    onContextMenu={(e) => handleContextMenu(e, entry)}
                    className={`flex flex-col items-center justify-center p-3 rounded-[var(--radius-md)] border text-center transition-all cursor-pointer relative group ${
                      isSelected
                        ? 'border-blue-500 bg-blue-500/10 shadow-xs'
                        : 'border-[var(--line)] bg-[var(--surface)] hover:border-blue-500/40 hover:bg-[var(--surface-hover)]'
                    }`}
                  >
                    <div className="absolute top-1.5 left-1.5 opacity-0 group-hover:opacity-100 transition-opacity">
                      <input
                        type="checkbox"
                        checked={isSelected}
                        onChange={() => {}}
                        className={`rounded border-[var(--line)] text-blue-600 pointer-events-none ${
                          isSelected ? 'opacity-100' : ''
                        }`}
                      />
                    </div>
                    <div className="h-10 w-10 flex items-center justify-center mb-2">
                      {React.cloneElement(getFileIcon(entry), { className: 'h-8 w-8 shrink-0' })}
                    </div>
                    <span className="text-xs font-medium truncate w-full" title={entry.name}>
                      {entry.name}
                    </span>
                    <span className="text-[10px] text-[var(--muted-foreground)] font-mono mt-0.5">
                      {entry.is_dir ? 'Folder' : formatFileSize(entry.size_bytes)}
                    </span>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>

      {/* 4. Floating Action Bar when Items are Selected */}
      {selectedPaths.size > 0 && (
        <div className="absolute bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-2 px-4 py-2 rounded-full border border-[var(--line)] bg-[var(--surface)] shadow-lg text-xs z-30 animate-in fade-in slide-in-from-bottom-2 duration-150">
          <span className="font-semibold px-2 py-0.5 rounded-full bg-blue-500/10 text-blue-500">
            {selectedPaths.size} terpilih
          </span>

          <button
            type="button"
            onClick={() => setIsCompressOpen(true)}
            className="flex items-center gap-1 px-2.5 py-1 rounded-full hover:bg-[var(--surface-hover)] font-medium"
          >
            <Archive className="h-3.5 w-3.5 text-purple-500" />
            <span>Arsipkan</span>
          </button>

          {selectedPaths.size === 1 && (
            <button
              type="button"
              onClick={() => {
                const targetPath = Array.from(selectedPaths)[0];
                const entry = entries.find((it) => it.path === targetPath);
                if (entry) openPermissionForEntry(entry);
              }}
              className="flex items-center gap-1 px-2.5 py-1 rounded-full hover:bg-[var(--surface-hover)] font-medium"
            >
              <ShieldAlert className="h-3.5 w-3.5 text-amber-500" />
              <span>Izin</span>
            </button>
          )}

          <button
            type="button"
            onClick={() => {
              setClipboard({ paths: Array.from(selectedPaths), operation: 'copy' });
            }}
            className="flex items-center gap-1 px-2.5 py-1 rounded-full hover:bg-[var(--surface-hover)] font-medium"
          >
            <Copy className="h-3.5 w-3.5" />
            <span>Salin</span>
          </button>

          <button
            type="button"
            onClick={() => {
              setClipboard({ paths: Array.from(selectedPaths), operation: 'cut' });
            }}
            className="flex items-center gap-1 px-2.5 py-1 rounded-full hover:bg-[var(--surface-hover)] font-medium"
          >
            <Scissors className="h-3.5 w-3.5" />
            <span>Potong</span>
          </button>

          <button
            type="button"
            onClick={() => setIsDeleteConfirmOpen(true)}
            className="flex items-center gap-1 px-2.5 py-1 rounded-full hover:bg-red-500/10 text-red-500 font-medium"
          >
            <Trash2 className="h-3.5 w-3.5" />
            <span>Hapus</span>
          </button>

          <button
            type="button"
            onClick={() => setSelectedPaths(new Set())}
            className="p-1 rounded-full hover:bg-[var(--surface-hover)] text-[var(--muted-foreground)] ml-1"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      {/* 5. Upload Tray (Right Bottom Floating Widget) */}
      {uploadQueue.length > 0 && (
        <div className="absolute right-4 bottom-4 w-80 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-lg z-40 overflow-hidden text-xs">
          <div className="flex items-center justify-between px-3 py-2 bg-[var(--surface-hover)] border-b border-[var(--line)]">
            <div className="flex items-center gap-1.5 font-semibold">
              <Upload className="h-3.5 w-3.5 text-blue-500" />
              <span>Tray Unggah ({uploadQueue.length})</span>
            </div>
            <div className="flex items-center gap-1">
              <button
                type="button"
                onClick={() => setIsUploadTrayMinimized(!isUploadTrayMinimized)}
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
                    <div className="w-full h-1.5 rounded-full bg-[var(--surface-hover)] overflow-hidden">
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
          className="fixed z-50 w-48 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-lg py-1 text-xs animate-in fade-in zoom-in-95 duration-100"
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
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsNewFolderOpen(false)}
        >
          <form
            onSubmit={handleCreateFolder}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Buat Folder Baru</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Folder</label>
                <input
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
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsNewFileOpen(false)}
        >
          <form
            onSubmit={handleCreateFile}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Buat Berkas Baru</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Berkas</label>
                <input
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
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsRenameOpen(false)}
        >
          <form
            onSubmit={handleRename}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Ganti Nama Berkas</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Baru</label>
                <input
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
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
          onDismiss={() => setIsCompressOpen(false)}
        >
          <form
            onSubmit={handleCompress}
            className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-5 shadow-xl space-y-4 dialog-panel"
          >
            <h3 className="text-sm font-semibold text-[var(--foreground)] dialog-header">Kompresi Menjadi Arsip</h3>
            <div className="dialog-body space-y-4">
              <div>
                <label className="text-xs text-[var(--muted-foreground)] block mb-1">Nama Arsip</label>
                <input
                  type="text"
                  value={compressName}
                  onChange={(e) => setCompressName(e.target.value)}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none"
                />
              </div>
              <div>
                <label className="text-xs text-[var(--muted-foreground)] block mb-1">Format</label>
                <select
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
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
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
        <ModalOverlay
          className="fixed inset-0 z-50 flex justify-end bg-black/50 backdrop-blur-xs"
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
