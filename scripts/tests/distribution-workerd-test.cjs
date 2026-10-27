/* Real Cloudflare runtime with intercepted outbound requests; no provider/account. */
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Miniflare, convertV4MiniflareOptions } = require('miniflare');

test('release proxy executes in workerd and never follows an upstream redirect', { timeout: 30000 }, async () => {
  let mode = 'file';
  const calls = [];
  const missing = JSON.stringify({ statusCode: '404', error: 'not_found', message: 'Object not found', code: 'NoSuchKey' });
  const runtime = new Miniflare(convertV4MiniflareOptions({
    modules: true,
    script: fs.readFileSync(path.join(__dirname, '../../distribution/worker.mjs'), 'utf8'),
    compatibilityDate: '2026-10-26',
    cf: false,
    outboundService: async request => {
      calls.push({ url: request.url, method: request.method });
      assert.equal(new URL(request.url).origin, 'https://rrewfkbaipgbvakaekcy.supabase.co');
      assert.equal(request.headers.get('authorization'), null);
      assert.equal(request.headers.get('cookie'), null);
      if (mode === 'pointer') {
        const url = new URL(request.url);
        assert.equal(request.headers.get('cache-control'), 'no-cache');
        assert.deepEqual([...url.searchParams.keys()], ['cacheNonce']);
        assert.match(url.searchParams.get('cacheNonce'), /^[a-f0-9-]{36}$/);
        return new Response(request.method === 'HEAD' ? null : 'current pointer');
      }
      if (mode === 'redirect') return Response.redirect('https://blocked.example.test/private', 302);
      if (mode === 'denied') return new Response('fixture private detail', { status: 403 });
      if (mode === 'missing') return new Response(request.method === 'HEAD' ? null : missing, { status: 400 });
      if (mode === 'partial') {
        assert.equal(request.headers.get('range'), 'bytes=0-3');
        return new Response('file', { status: 206, headers: { 'content-range': 'bytes 0-3/20' } });
      }
      return new Response(request.method === 'HEAD' ? null : 'fixture checksum', { status: 200 });
    },
  }));
  const url = 'https://download.arelqon.com/qonpanel/releases/v0.1.0/SHA256SUMS';
  try {
    let response = await runtime.dispatchFetch(url, { headers: { Authorization: 'fixture secret', Cookie: 'fixture secret' } });
    assert.equal(response.status, 200);
    assert.equal(await response.text(), 'fixture checksum');
    assert.equal(calls.length, 1);

    calls.length = 0;
    for (const arch of ['x86_64', 'aarch64']) {
      const name = `qinpanel-v0.1.0-${arch}-unknown-linux-gnu.tar.gz`;
      response = await runtime.dispatchFetch(`https://download.arelqon.com/qonpanel/releases/v0.1.0/${name}`);
      assert.equal(response.status, 200);
      assert.ok(calls.at(-1).url.endsWith('/' + name));
      assert.equal(new URL(calls.at(-1).url).search, '');
      await response.text();
    }
    assert.equal(calls.length, 2);

    mode = 'pointer'; calls.length = 0;
    for (const name of ['install.sh', 'latest.json']) {
      for (let attempt = 0; attempt < 2; attempt++) {
        response = await runtime.dispatchFetch('https://download.arelqon.com/qonpanel/' + name);
        assert.equal(response.status, 200);
        assert.equal(response.headers.get('cache-control'), 'no-store');
        assert.equal(await response.text(), 'current pointer');
      }
    }
    assert.equal(new Set(calls.map(call => new URL(call.url).search)).size, 4);
    response = await runtime.dispatchFetch('https://download.arelqon.com/qonpanel/latest.json?cacheNonce=client');
    assert.equal(response.status, 404);
    assert.equal(calls.length, 4);
    await response.text();

    mode = 'partial'; calls.length = 0;
    response = await runtime.dispatchFetch(url, { headers: { Range: 'bytes=0-3' } });
    assert.equal(response.status, 206);
    assert.equal(response.headers.get('content-range'), 'bytes 0-3/20');
    assert.equal(await response.text(), 'file');

    mode = 'missing'; calls.length = 0;
    response = await runtime.dispatchFetch(url);
    assert.equal(response.status, 404);
    assert.equal(response.headers.get('cache-control'), 'no-store');
    await response.text();
    response = await runtime.dispatchFetch(url, { method: 'HEAD' });
    assert.equal(response.status, 404);
    assert.equal(await response.text(), '');
    assert.deepEqual(calls.map(value => value.method), ['GET', 'HEAD', 'GET']);

    calls.length = 0;
    response = await runtime.dispatchFetch('https://download.arelqon.com/qonpanel/latest.json', { method: 'HEAD' });
    assert.equal(response.status, 404);
    assert.equal(await response.text(), '');
    assert.deepEqual(calls.map(call => call.method), ['HEAD', 'GET']);
    assert.equal(calls[0].url, calls[1].url);
    assert.match(new URL(calls[0].url).searchParams.get('cacheNonce'), /^[a-f0-9-]{36}$/);

    for (mode of ['redirect', 'denied']) {
      calls.length = 0;
      response = await runtime.dispatchFetch(url);
      assert.equal(response.status, 502);
      assert.equal(await response.text(), 'Artifact temporarily unavailable');
      assert.equal(calls.length, 1);
    }
  } finally {
    await runtime.dispose();
  }
});
