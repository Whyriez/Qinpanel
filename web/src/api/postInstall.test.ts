import { afterEach, describe, expect, it, vi } from 'vitest';
import { errorMessage, request, setCsrfToken, getCsrfToken, login } from './client';
import { createFileClient } from './files';

afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });
describe('post-install API contracts', () => {
  it('clears the cached CSRF token on session expiration', async () => {
    setCsrfToken('expired-session');
    vi.stubGlobal('fetch',vi.fn().mockResolvedValue(new Response(JSON.stringify({error:'Session expired'}),{status:401})));
    await expect(request('/api/v1/files/list')).rejects.toThrow('Session expired');
    expect(getCsrfToken()).toBeNull();
  });
  it('prefers detailed backend errors to generic status titles', async () => {
    expect(errorMessage({detail:'Pilih tenant', title:'Bad Request'}, 'fallback')).toBe('Pilih tenant');
    const fetchMock=vi.fn().mockResolvedValue(new Response(JSON.stringify({error:'Identitas tenant belum tersedia'}),{status:400}));
    vi.stubGlobal('fetch',fetchMock);
    await expect(request('/api/v1/files/list')).rejects.toThrow('Identitas tenant belum tersedia');
  });
  it('scopes reads, writes and binary chunks to the selected tenant', async () => {
    const fetchMock=vi.fn().mockImplementation(async (url:string) => new Response(JSON.stringify(url.includes('/list') ? {entries:[{name:'index.php',relative_path:'public/index.php',uid:10042,gid:10042}],total_count:1,has_more:false} : {success:true,current_offset:3,completed:true})));
    vi.stubGlobal('fetch',fetchMock);setCsrfToken('session-csrf');
    const client=createFileClient('42');
    const listed=await client.fetchFileList({path:'public'});
    expect(listed.entries[0].path).toBe('public/index.php');
    expect(fetchMock.mock.calls[0][0]).toContain('tenant=42');
    await client.renameFile('old','new');
    expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toMatchObject({source:'old',destination:'new',tenant:'42'});
    expect(fetchMock.mock.calls[1][1].headers.get('X-CSRF-Token')).toBe('session-csrf');
    await client.uploadChunk('upload',0,new Blob(['abc']));
    expect(fetchMock.mock.calls[2][0]).toContain('tenant=42');
    expect(client.url('/api/v1/files/download?path=index.php')).toContain('&tenant=42');
  });
  it('does not leak target state between simultaneous file managers', async () => {
    expect(createFileClient('1').url('/api/v1/files/list')).toBe('/api/v1/files/list?tenant=1');
    expect(createFileClient('2').url('/api/v1/files/list')).toBe('/api/v1/files/list?tenant=2');
    expect(createFileClient().url('/api/v1/files/list')).toBe('/api/v1/files/list');
  });
  it('sends the backend two-factor field', async () => {
    const fetchMock=vi.fn().mockResolvedValue(new Response(JSON.stringify({user:{id:1},csrf_token:'fresh'})));
    vi.stubGlobal('fetch',fetchMock);
    await login('admin','secret','123456');
    expect(JSON.parse(fetchMock.mock.calls[0][1].body).code).toBe('123456');
  });
});
