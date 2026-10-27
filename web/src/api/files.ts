import { request as apiRequest } from './client';
import type { FsEntry, FsListResponse, TrashListResponse, TrashedItem } from '../types/files';

export function createFileClient(tenant?: string) {
  const url = (path: string) => !tenant ? path : `${path}${path.includes('?') ? '&' : '?'}tenant=${encodeURIComponent(tenant)}`;
  const request = <T,>(path: string, options: RequestInit = {}) => {
    if (tenant && options.body && typeof options.body === 'string') {
      return apiRequest<T>(path, {...options, body:JSON.stringify({...JSON.parse(options.body), tenant})});
    }
    return apiRequest<T>(url(path), options);
  };
  const fetchFileList = async (params: {path?:string; sort_by?:string; sort_order?:string; show_hidden?:boolean; offset?:number; limit?:number; search_query?:string; extension?:string}): Promise<FsListResponse> => {
    const q = new URLSearchParams();
    Object.entries(params).forEach(([key,value]) => { if(value!==undefined) q.set(key === 'search_query' ? 'search' : key,String(value)); });
    const response = await request<{entries:(FsEntry & {relative_path:string; uid?:number; gid?:number})[]; total_count:number; has_more:boolean}>(`/api/v1/files/list?${q}`);
    return {entries:response.entries.map(e=>({...e,path:e.relative_path || e.path,owner_user:e.owner_user || String(e.uid ?? ''), owner_group:e.owner_group || String(e.gid ?? '')})), total:response.total_count,has_more:response.has_more,current_path:params.path || ''};
  };
  const post = (path: string, body: object) => request<{success:boolean;message:string;job_id?:string}>(path,{method:'POST',body:JSON.stringify(body)});
  return {
    url, request, fetchFileList,
    createFolder: (path:string, recursive=true) => post('/api/v1/files/mkdir',{path,recursive}),
    renameFile: (source:string,destination:string,overwrite=false) => post('/api/v1/files/rename',{source,destination,overwrite}),
    moveFile: (source:string,destination:string,overwrite=false) => post('/api/v1/files/move',{source,destination,overwrite}),
    copyFile: (source:string,destination:string,overwrite=false,background=false) => post('/api/v1/files/copy',{source,destination,recursive:true,overwrite,background}),
    moveToTrash: (path:string) => post('/api/v1/files/trash',{path}),
    fetchTrashList: async ():Promise<TrashListResponse> => {
      const response=await request<{items:(TrashedItem & {trash_id:string;trashed_at:number})[]}>('/api/v1/files/trash');
      const items=response.items.map(i=>({...i,id:i.trash_id || i.id,deleted_at:i.trashed_at || i.deleted_at}));
      return {items,total_size_bytes:items.reduce((sum,i)=>sum+i.size_bytes,0),count:items.length};
    },
    restoreTrash: (trash_id:string) => post('/api/v1/files/trash/restore',{trash_id}),
    emptyTrash: async () => { const result=await request<{success:boolean;purged_count:number;bytes_freed:number;message:string}>('/api/v1/files/trash/empty',{method:'DELETE'}); return {...result,deleted_count:result.purged_count,reclaimed_bytes:result.bytes_freed}; },
    deleteFilePermanent:(path:string) => request<{success:boolean;message:string}>('/api/v1/files/permanent',{method:'DELETE',body:JSON.stringify({path})}),
    compressArchive:(sources:string[],targetArchive:string,format='zip')=>post('/api/v1/files/archive/compress',{source_paths:sources,destination:targetArchive,format}),
    extractArchive:(archive_path:string,targetDir='')=>post('/api/v1/files/archive/extract',{archive_path,destination_dir:targetDir}),
    initUpload: async (path:string,total_size:number) => {
      const response=await request<{upload_id:string}>('/api/v1/files/upload/init',{method:'POST',body:JSON.stringify({target_path:path,file_name:path.split('/').pop(),total_size})});
      return {...response,chunk_size:1024*1024};
    },
    uploadChunk: async (uploadId:string,offset:number,chunk:Blob) => {
      const response = await request<{current_offset:number;completed:boolean}>(`/api/v1/files/upload/${encodeURIComponent(uploadId)}/chunk?offset=${offset}`,{method:'POST',body:chunk,headers:{'Content-Type':'application/octet-stream'}});
      return {success:true,current_offset:response.current_offset,complete:response.completed};
    },
  };
}
