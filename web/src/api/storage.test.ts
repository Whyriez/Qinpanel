import { afterEach, describe, expect, it, vi } from 'vitest';
import { setCsrfToken } from './client';
import { storageAllocationId, fetchStorageTls, generateStorageTls, configureManagedStorage, allocateManagedBucket, issueStorageKey, deleteStorageObject, projectStorageEnvironment, safeDownload, saveStorageConnection, uploadStorageObject, validObjectKey, previewStorageObject, saveStorageRules, saveStorageAllocation, measureStorageUsage, type StoragePolicy, type StorageConnection } from './storage';
const connection: StorageConnection = { id: 'fixture-connection', owner_id: 2, site_id: 1, label: 'Project media', provider: 'compatible', endpoint: 'https://objects.example.test', region: 'us-east-1', addressing: 'path', bucket: 'project-media', revision: 4, verified_at: 1, has_credentials: true, created_at: 1, updated_at: 1 };
afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });
describe('S3 client contracts', () => {
  it('fetches private previews only through the authenticated same-origin proxy', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(new Uint8Array([137, 80, 78, 71]), { headers: { 'Content-Type': 'image/png' } })); vi.stubGlobal('fetch', fetch);
    const signal = new AbortController().signal;
    expect((await previewStorageObject(connection, 'photos/a + café.png', signal)).type).toBe('image/png');
    const [url, options] = fetch.mock.calls[0] as [string, RequestInit];
    expect(url).toMatch(/^\/api\/v1\/storage\/connections\/fixture-connection\/download\?/);
    const query = new URL(url, 'https://panel.invalid').searchParams;
    expect(query.get('key')).toBe('photos/a + café.png'); expect(query.get('preview')).toBe('true');
    expect(options).toMatchObject({ credentials: 'include', cache: 'no-store', redirect: 'error', signal });
  });
  it('rejects active preview types, oversized previews and server authorization errors', async () => {
    const signal = new AbortController().signal;
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('<svg/>', { headers: { 'Content-Type': 'image/svg+xml' } })));
    await expect(previewStorageObject(connection, 'file.svg', signal)).rejects.toThrow('Unsupported preview');
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(new Uint8Array(8 * 1048576 + 1), { headers: { 'Content-Type': 'image/png' } })));
    await expect(previewStorageObject(connection, 'file.png', signal)).rejects.toThrow('8 MiB');
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ error: 'Owner session required' }, { status: 403 })));
    await expect(previewStorageObject(connection, 'file.png', signal)).rejects.toThrow('Owner session required');
  });
  it('binds rules and allocation updates to policy revisions without submitting object paths', async () => {
    const policy = { revision: 7, rules: { read: true, write: false, delete: false, max_file_bytes: 1048576, extensions: ['png'] } } as StoragePolicy;
    const fetch = vi.fn().mockImplementation(async () => Response.json(policy)); vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    await saveStorageRules(connection.id, policy, policy.rules);
    await saveStorageAllocation(connection.id, policy, 1073741824);
    await measureStorageUsage(connection.id);
    const calls = fetch.mock.calls as unknown as [string, RequestInit][];
    expect(JSON.parse(String(calls[0][1].body))).toEqual({ expected_revision: 7, rules: policy.rules });
    expect(JSON.parse(String(calls[1][1].body))).toEqual({ expected_revision: 7, quota_bytes: 1073741824 });
    expect(calls[2][0]).toContain('/usage');
    for (const [, options] of calls) expect(new Headers(options.headers).get('X-CSRF-Token')).toBe('fixture-csrf');
  });
  it('keeps unavailable DNS status distinct and accepts legacy inventory responses', async () => {
    for (const dns of [{ configured: false, zone_name: null }, { available: false, configured: false, zone_name: null }, { available: true, configured: true, zone_name: 'example.test' }]) {
      const value = { certificate: { revision: 1, state: 'idle', auto_renew: false }, dns };
      const fetch = vi.fn().mockResolvedValue(Response.json(value)); vi.stubGlobal('fetch', fetch);
      expect(await fetchStorageTls()).toEqual(value);
      expect(fetch.mock.calls[0][0]).toBe('/api/v1/storage/managed/tls');
      expect(fetch.mock.calls[0][1].method).toBeUndefined();
    }
  });
  it('rejects malformed DNS availability instead of exposing an unsafe generate action', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ certificate: { revision: 1, state: 'idle', auto_renew: false }, dns: { available: 'yes', configured: true } })));
    await expect(fetchStorageTls()).rejects.toThrow('Status sertifikat storage tidak valid');
  });
  it('requests automated TLS through an authenticated, revision-bound operation without PEM or provider keys', async () => {
    const fetch=vi.fn(async()=>new Response('{}',{status:202})); vi.stubGlobal('fetch',fetch); setCsrfToken('fixture-csrf');
    const request={request_id:storageAllocationId(),expected_revision:1,hostname:'storage.example.test',email:'admin@example.test',confirmed:true};
    await generateStorageTls(request);
    const [url,options]=fetch.mock.calls[0] as unknown as [string,RequestInit];
    expect(url).toBe('/api/v1/storage/managed/tls'); expect(JSON.parse(options.body as string)).toEqual(request);
    expect(new Headers(options.headers).get('X-CSRF-Token')).toBe('fixture-csrf'); expect(options.body).not.toContain('private_key');
  });
  it('uses HTTP-compatible random allocation UUIDs and binds key rotation to its generation', async () => {
    expect(storageAllocationId()).toMatch(/^[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/);
    expect(storageAllocationId()).not.toBe(storageAllocationId());
    const fetch = vi.fn(async () => new Response('{}', { status: 200 })); vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    await issueStorageKey({ ...connection, managed: true, application_key_generation: 3 }, connection.bucket, true);
    const [url, options] = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toContain('/application-key'); expect(JSON.parse(options.body as string)).toEqual({ expected_revision: 4, expected_generation: 3, confirmation: 'project-media', retry: true });
    expect(new Headers(options.headers).get('X-CSRF-Token')).toBe('fixture-csrf');
  });
  it('keeps private certificate material in the authenticated POST body only', async () => {
    const fetch = vi.fn(async () => new Response('{}', { status: 200 })); vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    await configureManagedStorage({ hostname: 'storage.example.test', fullchain_pem: 'fixture-chain', private_key_pem: 'fixture-private-key' });
    const [url, options] = fetch.mock.calls[0] as unknown as [string, RequestInit]; expect(url).toBe('/api/v1/storage/managed'); expect(url).not.toContain('fixture-private-key'); expect(options.body).toContain('fixture-private-key');
    await allocateManagedBucket({ request_id: storageAllocationId(), owner_id: 2, site_id: 1, label: 'Media', bucket: 'project-media' });
    const [secondUrl] = fetch.mock.calls[1] as unknown as [string, RequestInit];
    expect(secondUrl).toBe('/api/v1/storage/managed/buckets');
  });

  it.each(['../secret', 'a/./b', 'a\\b', 'a\nb', '', 'a'.repeat(1025)])('rejects unsupported object key %j', value => expect(validObjectKey(value)).toBe(false));
  it('preserves portable Unicode and literal percent/plus keys', () => { expect(validObjectKey('images/foto + café.png')).toBe(true); expect(validObjectKey('literal%2f.png')).toBe(true); });
  it('only accepts a fresh HTTPS download on the configured bucket origin', () => {
    const value = { url: 'https://objects.example.test/project-media/file.txt?X-Amz-Signature=fixture', expires_at: Math.floor(Date.now() / 1000) + 60 };
    expect(safeDownload(value, connection)).toContain('/project-media/file.txt');
    for (const url of ['http://objects.example.test/project-media/a', 'https://evil.test/project-media/a', 'https://objects.example.test/another-bucket/a', 'https://user:secret@objects.example.test/project-media/a']) expect(() => safeDownload({ ...value, url }, connection)).toThrow();
    expect(() => safeDownload({ ...value, expires_at: 1 }, connection)).toThrow();
  });
  it('accepts only session-authenticated proxy URLs for the same connection and revision', () => {
    const expires_at = Math.floor(Date.now() / 1000) + 60;
    const url = `/api/v1/storage/connections/${connection.id}/download?expected_revision=4&key=file.txt&expires_at=${expires_at}`;
    expect(safeDownload({ url, expires_at }, connection)).toBe(url);
    expect(() => safeDownload({ url: url.replace('expected_revision=4', 'expected_revision=3'), expires_at }, connection)).toThrow();
    expect(() => safeDownload({ url: url.replace(connection.id, 'other-connection'), expires_at }, connection)).toThrow();
  });
  it('keeps credentials in the CSRF protected JSON body and never in the URL', async () => {
    const fetch = vi.fn(async () => new Response(JSON.stringify(connection), { status: 200 })); vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    await saveStorageConnection({ expected_revision: 0, owner_id: 2, site_id: 1, label: 'Media', provider: 'compatible', endpoint: connection.endpoint, bucket: connection.bucket, region: connection.region, addressing: 'path', credentials: { access_key: 'fixture_access', secret_key: 'fixture_secret', session_token: null } });
    const [url, options] = fetch.mock.calls[0] as unknown as [string, RequestInit]; expect(url).not.toContain('fixture_secret'); expect(options.body).toContain('fixture_secret'); expect(new Headers(options.headers).get('X-CSRF-Token')).toBe('fixture-csrf');
  });
  it('binds destructive confirmation to the exact object and connection revision', async () => {
    const fetch = vi.fn(async () => new Response('{}')); vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    await deleteStorageObject(connection, 'images/foto.png');
    const [, options] = fetch.mock.calls[0] as unknown as [string, RequestInit]; expect(JSON.parse(String(options.body))).toEqual({ expected_revision: 4, key: 'images/foto.png', confirmation: 'images/foto.png' });
  });
  it('exports only explicit application configuration with placeholder keys', () => {
    const env = projectStorageEnvironment(connection); expect(env).toContain('AWS_BUCKET=project-media'); expect(env).toContain('AWS_USE_PATH_STYLE_ENDPOINT=true'); expect(env).toContain('<secret-access-key>'); expect(env).not.toContain('enc:v1:');
  });
  it('rejects oversized uploads before opening a network request', async () => {
    const xhr = vi.fn(); vi.stubGlobal('XMLHttpRequest', xhr);
    await expect(uploadStorageObject(connection, { size: 33554433 } as File, 'file.bin', false, vi.fn(), new AbortController().signal)).rejects.toThrow('32 MiB'); expect(xhr).not.toHaveBeenCalled();
  });
});
