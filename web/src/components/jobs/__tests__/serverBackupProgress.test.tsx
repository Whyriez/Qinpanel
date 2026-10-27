import { expect, it } from 'vitest';
import { canCancelJob, jobProgressLabel, jobSnapshotEvent, jobTitle } from '../../../lib/jobProgress';
import type { JobItem } from '../../../types/api';

const job:JobItem={id:'fixture',user_id:1,name:'server.backup.create',status:'running',progress:40,payload_json:'{"external":true,"operation":"create"}',result_json:'{"message":"Memeriksa integritas arsip.","progress_known":true}',error_message:null,retry_count:0,max_retries:0,created_at:1,updated_at:2};
it('renders the observed root stage and never offers generic queue cancellation',()=>{
 expect(jobTitle(job.name)).toBe('Backup penuh server');expect(jobProgressLabel(job)).toBe('40%');expect(jobSnapshotEvent(job).message).toBe('Memeriksa integritas arsip.');
 for(const operation of ['create','inspect','restore'])for(const status of ['queued','running'] as const)expect(canCancelJob({...job,name:`server.backup.${operation}`,status})).toBe(false);
 expect(jobProgressLabel({...job,progress:0,result_json:'{"progress_known":false}'})).toBe('Menunggu status tahapan');
});
it('rejects malformed display metadata and leaves ordinary job handling intact',()=>{
 expect(jobSnapshotEvent({...job,result_json:'{"message":{"unexpected":1}}'}).message).toContain('sedang berjalan');
 expect(canCancelJob({...job,name:'system.echo'})).toBe(true);expect(jobTitle('system.echo')).toBe('system.echo');
});
