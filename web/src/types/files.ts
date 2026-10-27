export interface FsEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size_bytes: number;
  modified_at: number;
  mode: number;
  owner_user: string;
  owner_group: string;
  is_symlink?: boolean;
  symlink_target?: string | null;
  is_hidden?: boolean;
}

export interface FsListResponse {
  entries: FsEntry[];
  total: number;
  has_more: boolean;
  current_path: string;
}

export interface TrashedItem {
  id: string;
  original_path: string;
  deleted_at: number;
  size_bytes: number;
  is_dir: boolean;
}

export interface TrashListResponse {
  items: TrashedItem[];
  total_size_bytes: number;
  count: number;
}

export interface UploadItem {
  id: string;
  file: File;
  targetPath: string;
  totalSize: number;
  uploadedSize: number;
  status: 'pending' | 'uploading' | 'paused' | 'completed' | 'error';
  errorMessage?: string;
  uploadId?: string;
}

export interface DiskQuotaStatus {
  usedBytes: number;
  limitBytes: number;
  percentage: number;
}

export type ViewMode = 'list' | 'grid';
export type SortField = 'name' | 'size' | 'modified' | 'extension';
export type SortDirection = 'asc' | 'desc';
