import { test } from 'node:test';
import assert from 'node:assert/strict';
import worker from '../../distribution/worker.mjs';

const missingKey = { statusCode: '404', error: 'not_found', message: 'Object not found', code: 'NoSuchKey' };

test('Supabase HTTP 400 missing keys become uncached 404 without exposing upstream bodies', async () => {
  const original = globalThis.fetch;
  try {
    for (const body of [missingKey, { statusCode: 404, code: 'NoSuchKey' }, { statusCode: '404', error: 'not_found', message: 'Object not found' }]) {
      globalThis.fetch = async () => new Response(JSON.stringify(body), { status: 400 });
      for (const path of ['/qonpanel/latest.json', '/qonpanel/releases/v0.1.0/SHA256SUMS']) {
        const response = await worker.fetch(new Request('https://download.arelqon.com' + path), {});
        assert.equal(response.status, 404);
        assert.equal(response.headers.get('cache-control'), 'no-store');
        assert.equal(await response.text(), 'Not found');
      }
    }
  } finally { globalThis.fetch = original; }
});

test('HEAD 400 is confirmed by a single credential-free GET of the same object', async () => {
  const original = globalThis.fetch;
  try {
    for (const probeStatus of [400, 404, 200, 403]) {
      const calls = [];
      globalThis.fetch = async (url, options) => {
        calls.push({ url, options });
        return options.method === 'HEAD'
          ? new Response(null, { status: 400 })
          : new Response(JSON.stringify(missingKey), { status: probeStatus });
      };
      const response = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/latest.json', {
        method: 'HEAD', headers: { Range: 'bytes=0-3', Authorization: 'fixture_secret', Cookie: 'fixture_secret', 'If-None-Match': 'fixture' },
      }), {});
      assert.equal(response.status, [400, 404].includes(probeStatus) ? 404 : 502);
      assert.equal(await response.text(), '');
      assert.equal(calls.length, 2);
      assert.equal(calls[0].url, calls[1].url);
      assert.equal(calls[1].options.method, 'GET');
      assert.equal(calls[1].options.redirect, 'manual');
      const headers = new Headers(calls[1].options.headers);
      for (const name of ['authorization', 'cookie', 'range', 'if-none-match']) assert.equal(headers.get(name), null);
      assert.equal(calls[1].options.cf.cacheTtlByStatus['100-599'], -1);
    }
  } finally { globalThis.fetch = original; }
});

test('authorization, bucket, invalid and oversized error envelopes remain failures', async () => {
  const original = globalThis.fetch;
  try {
    for (const body of [
      { statusCode: '403', code: 'AccessDenied' },
      { statusCode: '404', code: 'NoSuchBucket' },
      { ...missingKey, code: 'InvalidJWT' },
      { statusCode: '404', error: 'not_found', message: 'Bucket not found' },
      null, [], 'not JSON', ' '.repeat(4097),
    ]) {
      globalThis.fetch = async () => new Response(typeof body === 'string' ? body : JSON.stringify(body), { status: 400 });
      const response = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/latest.json'), {});
      assert.equal(response.status, 502);
      assert.equal(await response.text(), 'Artifact temporarily unavailable');
    }
  } finally { globalThis.fetch = original; }
});

test('only release downloads are proxied; arbitrary paths, queries and writes stay blocked', async () => {
  let calls = 0;
  const original = globalThis.fetch;
  globalThis.fetch = async () => { calls++; return new Response('unexpected'); };
  try {
    for (const path of ['/qonpanel', '/', '/qonpanel/.env', '/qonpanel/latest.json?url=https://evil.test', '/qonpanel/releases/v1.0.0/secret.txt', '/qonpanel/releases/v1.0.0/qonpanel-v2.0.0-x86_64-unknown-linux-gnu.tar.gz', '/qonpanel/%2eenv', '/qonpanel/releases/v1.0.0/source.zip']) {
      assert.equal((await worker.fetch(new Request('https://download.arelqon.com' + path), {})).status, 404);
    }
    assert.equal((await worker.fetch(new Request('https://download.arelqon.com/qonpanel/install.sh', { method: 'POST', body: 'x' }), {})).status, 405);
    assert.equal(calls, 0);
  } finally { globalThis.fetch = original; }
});

test('canonical Qinpanel downloads and historical aliases remain allowlisted for both architectures', async () => {
  const original = globalThis.fetch;
  const calls = [];
  try {
    globalThis.fetch = async (url, options) => {
      calls.push(url);
      assert.equal(new URL(url).search, '');
      return new Response(options.method === 'HEAD' ? null : 'package');
    };
    for (const brand of ['qinpanel', 'qonpanel', 'nusa-panel']) {
      for (const arch of ['x86_64', 'aarch64']) {
        for (const method of ['GET', 'HEAD']) {
          const name = `${brand}-v0.1.0-${arch}-unknown-linux-gnu.tar.gz`;
          const response = await worker.fetch(new Request(`https://download.arelqon.com/qonpanel/releases/v0.1.0/${name}`, { method }), {});
          assert.equal(response.status, 200);
          assert.match(response.headers.get('cache-control'), /immutable/);
          assert.ok(calls.at(-1).endsWith('/releases/v0.1.0/' + name));
          assert.equal(await response.text(), method === 'HEAD' ? '' : 'package');
        }
      }
    }
    assert.equal(calls.length, 12);
    for (const name of ['qinpanel-v0.2.0-x86_64-unknown-linux-gnu.tar.gz', 'qinpanel-v0.1.0-riscv64-unknown-linux-gnu.tar.gz']) {
      assert.equal((await worker.fetch(new Request(`https://download.arelqon.com/qonpanel/releases/v0.1.0/${name}`), {})).status, 404);
    }
    assert.equal(calls.length, 12);
  } finally { globalThis.fetch = original; }
});

test('streamed downloads preserve ranges without forwarding credentials or cookies', async () => {
  const original = globalThis.fetch;
  globalThis.fetch = async (url, options) => {
    assert.match(url, /^https:\/\/rrewfkbaipgbvakaekcy\.supabase\.co\/storage\/v1\/object\/public\/qonpanel-releases\/releases\//);
    assert.equal(options.headers.get('range'), 'bytes=0-3');
    assert.equal(options.headers.get('authorization'), null);
    assert.equal(options.headers.get('cookie'), null);
    assert.equal(options.redirect, 'manual');
    return new Response('file', { status: 206, headers: { 'content-range': 'bytes 0-3/20', 'set-cookie': 'private', 'content-type': 'application/gzip' } });
  };
  try {
    const response = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/releases/v1.0.0/qonpanel-v1.0.0-x86_64-unknown-linux-gnu.tar.gz', { headers: { Range: 'bytes=0-3', Authorization: 'secret', Cookie: 'secret' } }), {});
    assert.equal(response.status, 206);
    assert.equal(response.headers.get('content-range'), 'bytes 0-3/20');
    assert.equal(response.headers.get('set-cookie'), null);
    assert.match(response.headers.get('cache-control'), /immutable/);
    assert.equal(await response.text(), 'file');
  } finally { globalThis.fetch = original; }
});

test('latest and installer bypass cache; HEAD, outages and config errors are bounded', async () => {
  const original = globalThis.fetch;
  try {
    for (const name of ['install.sh', 'latest.json']) {
      globalThis.fetch = async (_, options) => {
        assert.equal(options.cf.cacheTtlByStatus['100-599'], -1);
        return new Response('metadata', { headers: { 'content-type': 'text/plain' } });
      };
      const response = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/' + name, { method: 'HEAD' }), {});
      assert.equal(response.headers.get('cache-control'), 'no-store');
      assert.equal(await response.text(), '');
    }
    globalThis.fetch = async () => { throw new Error('private upstream detail'); };
    const failed = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/latest.json'), {});
    assert.equal(failed.status, 502);
    assert.ok(!(await failed.text()).includes('private'));
    assert.equal((await worker.fetch(new Request('https://download.arelqon.com/qonpanel/latest.json'), { SUPABASE_PUBLIC_BASE_URL: 'https://evil.test' })).status, 503);
  } finally { globalThis.fetch = original; }
});

test('mutable pointers use fresh internal Supabase nonces while canonical public URLs stay query-free', async () => {
  const original = globalThis.fetch;
  const upstreams = [];
  try {
    globalThis.fetch = async (url, options) => {
      const upstream = new URL(url);
      upstreams.push(upstream);
      assert.equal(upstream.origin, 'https://rrewfkbaipgbvakaekcy.supabase.co');
      assert.equal(options.headers.get('cache-control'), 'no-cache');
      assert.equal(options.headers.get('authorization'), null);
      assert.equal(options.headers.get('cookie'), null);
      assert.equal(options.cf.cacheTtlByStatus['100-599'], -1);
      assert.equal(options.cf.cacheEverything, false);
      assert.deepEqual([...upstream.searchParams.keys()], ['cacheNonce']);
      assert.match(upstream.searchParams.get('cacheNonce'), /^[a-f0-9-]{36}$/);
      return new Response('current pointer');
    };
    for (const name of ['install.sh', 'latest.json']) {
      for (let attempt = 0; attempt < 2; attempt++) {
        const response = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/' + name, {
          headers: { Authorization: 'fixture_secret', Cookie: 'fixture_secret', 'Cache-Control': 'public, max-age=99999' },
        }), {});
        assert.equal(response.status, 200);
        assert.equal(response.headers.get('cache-control'), 'no-store');
        assert.equal(await response.text(), 'current pointer');
      }
    }
    assert.equal(new Set(upstreams.map(url => url.search)).size, 4);
    assert.deepEqual(upstreams.map(url => url.pathname.split('/').at(-1)), ['install.sh', 'install.sh', 'latest.json', 'latest.json']);
    assert.equal((await worker.fetch(new Request('https://download.arelqon.com/qonpanel/latest.json?cacheNonce=client'), {})).status, 404);
    assert.equal(upstreams.length, 4);
  } finally { globalThis.fetch = original; }
});

test('negative and partial responses never poison immutable artifact cache', async () => {
  const original = globalThis.fetch;
  try {
    for (const status of [404, 206]) {
      globalThis.fetch = async (url, options) => {
        assert.equal(new URL(url).search, '');
        assert.equal(options.cf.cacheTtlByStatus['201-599'], -1);
        assert.equal(options.cf.cacheTtlByStatus[200], 86400);
        return new Response(status === 404 ? 'missing' : 'part', { status });
      };
      const response = await worker.fetch(new Request('https://download.arelqon.com/qonpanel/releases/v1.0.0/SHA256SUMS'), {});
      assert.equal(response.status, status);
      if (status === 404) assert.equal(response.headers.get('cache-control'), 'no-store');
    }
  } finally { globalThis.fetch = original; }
});
