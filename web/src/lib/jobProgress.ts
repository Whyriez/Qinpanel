import type { JobItem, JobProgressEvent } from '../types/api';

export function canCancelJob(job: JobItem): boolean {
  if (isServerBackupJob(job.name)) return false;
  return job.status === 'queued' || (job.status === 'running' && !['php.install', 'php.repo.setup', 'database.server.install', 'database.web.install', 'cloudflare.install', 'cloudflare.change', 'email.install', 'email.mailbox', 'email.send', 'email.deliver', 'panel.tls.issue', 'ssl.dns01.issue', 'storage.tls.issue'].includes(job.name));
}

export const isServerBackupJob = (name: string) => ['server.backup.create', 'server.backup.inspect', 'server.backup.restore'].includes(name);
export function jobTitle(name: string): string {
  return ({'server.backup.create':'Backup penuh server','server.backup.inspect':'Pemeriksaan arsip backup','server.backup.restore':'Pemulihan server'} as Record<string,string>)[name] ?? name;
}
export function serverBackupJobDetail(job: JobItem): { message?: string; progress_known?: boolean } {
  if (!isServerBackupJob(job.name)) return {};
  try { const value:unknown=JSON.parse(job.result_json ?? '{}'); if(!value || typeof value!=='object') return {}; const fields=value as Record<string,unknown>; return {message:typeof fields.message==='string'?fields.message:undefined,progress_known:typeof fields.progress_known==='boolean'?fields.progress_known:undefined}; } catch {return {};}
}
export function jobProgressLabel(job: JobItem): string {
  return isServerBackupJob(job.name) && serverBackupJobDetail(job).progress_known===false && ['queued','running'].includes(job.status) ? 'Menunggu status tahapan' : `${job.progress}%`;
}

export function jobProgressMessage(status: JobProgressEvent['status'], message: string | null): string {
  if (message) return message;
  switch (status) {
    case 'queued': return 'Menunggu giliran eksekusi. Tugas belum dimulai.';
    case 'running': return 'Tugas sedang berjalan. Menunggu pembaruan progres.';
    case 'succeeded': return 'Tugas selesai.';
    case 'failed': return 'Tugas gagal.';
    case 'cancelled': return 'Tugas dibatalkan.';
  }
}

export function jobSnapshotEvent(job: JobItem): JobProgressEvent {
  return {
    job_id: job.id, status: job.status, progress: job.progress,
    message: jobProgressMessage(job.status, serverBackupJobDetail(job).message ?? (job.status === 'running' || job.status === 'succeeded' ? null : job.error_message)), timestamp: job.updated_at,
  };
}

/** Stable server sequences preserve equal-timestamp output and deduplicate reconnects. */
export function mergeJobEvents(previous: JobProgressEvent[], incoming: JobProgressEvent[]): JobProgressEvent[] {
  const all = [...previous, ...incoming];
  const persisted = all.some(event => event.sequence !== undefined);
  const map = new Map<string, JobProgressEvent>();
  for (const event of persisted ? all.filter(item => item.sequence !== undefined) : all) {
    const key = event.sequence !== undefined ? `seq:${event.sequence}` : JSON.stringify([event.job_id, event.status, event.progress, event.timestamp, event.message]);
    map.set(key, event);
  }
  return [...map.values()].sort((a, b) => a.sequence !== undefined && b.sequence !== undefined ? a.sequence - b.sequence : a.timestamp - b.timestamp).slice(-5000);
}
