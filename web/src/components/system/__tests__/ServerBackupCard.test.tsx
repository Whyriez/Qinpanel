import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ServerBackupSummary, ServerRestoreRequirements } from '../ServerBackupCard';
import { archiveIsFresh, parseServerBackupStatus, restoreApproved, serverBackupId, type ServerBackupStatus } from '../../../api/serverBackup';
import '../../../i18n';
import { ServerBackupDialog } from '../ServerBackupDialog';
const data:ServerBackupStatus={supported:true,job:{id:'fixture',operation:'create',phase:'running',message:'<script>secret()</script>'},backups:[],inspection:null};
describe('Server backup and migration',()=>{
 it('uses an accessible confirmation with explicit action and snapshot downtime details',()=>{
  const html=renderToStaticMarkup(<ServerBackupDialog operation="create" submitting={false} error="" adminPassword="" accepted={false} onPassword={()=>{}} onAccept={()=>{}} onClose={()=>{}} onSubmit={()=>{}}/>);
  expect(html).toContain('Buat backup server');expect(html).toContain('aria-labelledby');expect(html).toContain('Jeda layanan sementara');expect(html).toContain('koneksi Tunnel');expect(html).toContain('Tugas');expect(html).not.toContain('Lanjutkan');
 });
 it('shows measured stage progress and a direct Tasks link without inventing progress for old workers',()=>{
  const html=renderToStaticMarkup(<ServerBackupSummary data={{...data,job:{...data.job!,progress:40}}}/>);
  expect(html).toContain('40');expect(html).toContain('href="/jobs"');expect(html).toContain('terputus sementara');
  expect(renderToStaticMarkup(<ServerBackupSummary data={data}/>)).not.toContain('Progres tahapan');
  for(const progress of [-1,101,0.5,'40'])expect(()=>parseServerBackupStatus({...data,job:{...data.job,progress}})).toThrow();
 });
 it('explains matching OS releases and both package formats in Indonesian and English',()=>{
  for(const english of [false,true]){
   const html=renderToStaticMarkup(<ServerRestoreRequirements english={english}/>);
   for(const platform of ['Debian 12','Ubuntu 22.04/24.04','AlmaLinux 9/10','APT','DNF'])expect(html).toContain(platform);
   expect(html).toContain(english?'Cross-distribution restores are not supported':'Restore lintas distro tidak didukung');
   expect(html).toContain(english?'same or newer':'sama atau lebih baru');
  }
 });
 it('creates canonical unique request IDs without requiring secure-context randomUUID',()=>{const first=serverBackupId();expect(first).toMatch(/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/);expect(serverBackupId()).not.toBe(first);});
 it('announces durable jobs and escapes diagnostic text',()=>{const html=renderToStaticMarkup(<ServerBackupSummary data={data}/>);expect(html).toContain('role="status"');expect(html).toContain('Pekerjaan berjalan');expect(html).toContain('&lt;script&gt;');expect(html).not.toContain('<script>');});
 it('explains managed scope and missing installation support',()=>{const html=renderToStaticMarkup(<ServerBackupSummary data={{...data,supported:false,job:null}}/>);expect(html).toContain('Upgrade paket');expect(html).toContain('layanan eksternal');});
 it('requires a matching recent inspection and valid digest before restore',()=>{const report={archive_id:'fixture',digest:'a'.repeat(64),hostname:'old',panel_version:'0.1.0',tenant_count:1,uncompressed_bytes:1,packages:[],checked_at:2000,warnings:[]};expect(archiveIsFresh(report,'fixture',2000_000)).toBe(true);expect(archiveIsFresh(report,'other',2000_000)).toBe(false);expect(archiveIsFresh(report,'fixture',4000_000)).toBe(false);expect(archiveIsFresh({...report,checked_at:2001},'fixture',2000_000)).toBe(false);});
 it('requires a live root approval bound to the inspected archive',()=>{const report={archive_id:'fixture',digest:'a'.repeat(64),hostname:'old',panel_version:'0.1.0',tenant_count:1,uncompressed_bytes:1,packages:[],checked_at:2000,warnings:[]};expect(restoreApproved({...data,inspection:report},2000_000)).toBe(false);expect(restoreApproved({...data,inspection:report,approval:{digest:report.digest,expires_at:2001}},2000_000)).toBe(true);expect(restoreApproved({...data,inspection:report,approval:{digest:'b'.repeat(64),expires_at:2001}},2000_000)).toBe(false);});
 it('rejects malformed status and accepts an interrupted job without replay',()=>{expect(()=>parseServerBackupStatus({})).toThrow();expect(()=>parseServerBackupStatus({...data,job:{phase:'unknown'}})).toThrow();expect(parseServerBackupStatus({...data,job:{...data.job!,phase:'interrupted'}}).job?.phase).toBe('interrupted');});
});
