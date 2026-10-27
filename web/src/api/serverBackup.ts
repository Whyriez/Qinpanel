import { request } from './client';
export type BackupOperation = 'create' | 'inspect' | 'restore';
export interface ServerArchive { id: string; size: number; created_at: number; digest: string; hostname: string; panel_version: string }
export interface ArchiveInspection { archive_id: string; digest: string; hostname: string; panel_version: string; tenant_count: number; uncompressed_bytes: number; packages: string[]; checked_at: number; warnings: string[]; public_port?: number }
export interface ServerBackupStatus { supported: boolean; approval?: {digest:string;expires_at:number}|null; job: null | { id: string; operation: BackupOperation; phase: 'queued'|'running'|'succeeded'|'failed'|'interrupted'; message: string; progress?: number }; backups: ServerArchive[]; inspection: ArchiveInspection | null }
export interface BackupJob { id: string; operation: BackupOperation; passphrase: string; archive_id?: string; digest?: string; new_ipv4?: string; confirm_restore?: boolean; admin_password?: string }
// getRandomValues also works on the initial HTTP/IP installation, unlike randomUUID.
export function serverBackupId(): string {
  const bytes=crypto.getRandomValues(new Uint8Array(16));
  bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;
  const hex=Array.from(bytes,value=>value.toString(16).padStart(2,'0')).join('');
  return `${hex.slice(0,8)}-${hex.slice(8,12)}-${hex.slice(12,16)}-${hex.slice(16,20)}-${hex.slice(20)}`;
}
export function parseServerBackupStatus(value: unknown): ServerBackupStatus {
  if (!value || typeof value !== 'object') throw new Error('Status backup tidak valid.');
  const data = value as ServerBackupStatus;
  if (typeof data.supported !== 'boolean' || !Array.isArray(data.backups) || (data.job != null && (!['queued','running','succeeded','failed','interrupted'].includes(data.job.phase) || typeof data.job.message !== 'string' || (data.job.progress!==undefined && (!Number.isInteger(data.job.progress)||data.job.progress<0||data.job.progress>100)))) || (data.inspection != null && (!Array.isArray(data.inspection.packages) || !Array.isArray(data.inspection.warnings) || typeof data.inspection.digest !== 'string'))) throw new Error('Status backup tidak valid.');
  return data;
}
export const serverBackupStatus = async () => parseServerBackupStatus(await request<unknown>('/api/v1/system/server-backup'));
export const startServerBackup = async (job: BackupJob) => parseServerBackupStatus(await request<unknown>('/api/v1/system/server-backup/jobs', { method:'POST',body:JSON.stringify(job) }));
export const removeServerBackup = async (id:string) => parseServerBackupStatus(await request<unknown>(`/api/v1/system/server-backup/archives/${encodeURIComponent(id)}`,{method:'DELETE'}));
export const removeServerUpload = (id:string) => request(`/api/v1/system/server-backup/uploads/${encodeURIComponent(id)}`,{method:'DELETE'});
export function restoreApproved(data:ServerBackupStatus,now=Date.now()):boolean{return !!data.inspection&&!!data.approval&&data.approval.digest===data.inspection.digest&&data.approval.expires_at>=now/1000;}
export function archiveIsFresh(inspection:ArchiveInspection|null,id:string,now=Date.now()):boolean { return !!inspection && inspection.archive_id===id && now/1000>=inspection.checked_at && now/1000-inspection.checked_at<=1800 && /^[a-f0-9]{64}$/.test(inspection.digest); }
export async function uploadServerArchive(file:File,id:string,onProgress:(offset:number)=>void,signal:AbortSignal):Promise<void>{
  let offset=0;
  while(offset<file.size){
    const end=Math.min(offset+8*1024*1024,file.size);
    const result=await request<{offset:number;complete:boolean}>(`/api/v1/system/server-backup/uploads/${encodeURIComponent(id)}?offset=${offset}&total=${file.size}`,{method:'PUT',body:file.slice(offset,end),headers:{'Content-Type':'application/octet-stream'},signal});
    if (!Number.isSafeInteger(result.offset) || result.offset<0 || result.offset>file.size || result.offset===offset) throw new Error('Progres unggahan tidak valid. Pilih ulang berkas untuk mencoba kembali.');
    offset=result.offset; onProgress(offset);
  }
}
