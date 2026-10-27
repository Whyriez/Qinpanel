import { afterEach, describe, expect, it, vi } from 'vitest';
import { setCsrfToken } from './client';
import { storageAllocationId, generateStorageTls, configureManagedStorage, allocateManagedBucket, issueStorageKey, deleteStorageObject, projectStorageEnvironment, safeDownload, saveStorageConnection, uploadStorageObject, validObjectKey, type StorageConnection } from './storage';
const connection: StorageConnection = { id: 'fixture-connection', owner_id: 2, site_id: 1, label: 'Project media', provider: 'compatible', endpoint: 'https://objects.example.test', region: 'us-east-1', addressing: 'path', bucket: 'project-media', revision: 4, verified_at: 1, has_credentials: true, created_at: 1, updated_at: 1 };
afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });
describe('S3 client contracts', () => {
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
