// Public artifact proxy only. No GitHub or Supabase credentials are required.
const prefix = '/qonpanel';
const project = 'rrewfkbaipgbvakaekcy';

async function storageObjectMissing(response) {
  if (response.status === 404) {
    await response.body?.cancel();
    return true;
  }
  if (response.status !== 400 || !response.body) {
    await response.body?.cancel();
    return false;
  }
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 4096) return false;
      chunks.push(value);
    }
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    const value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      && String(value.statusCode) === '404'
      && (value.code === 'NoSuchKey' || !Object.hasOwn(value, 'code')
        && value.error === 'not_found' && value.message === 'Object not found');
  } catch {
    return false;
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!['GET', 'HEAD'].includes(request.method)) {
      return new Response('Method not allowed', { status: 405, headers: { Allow: 'GET, HEAD' } });
    }
    const name = url.pathname.startsWith(prefix + '/') ? url.pathname.slice(prefix.length + 1) : '';
    const version = 'v(?:0|[1-9][0-9]{0,7})\\.(?:0|[1-9][0-9]{0,7})\\.(?:0|[1-9][0-9]{0,7})';
    const release = new RegExp(`^releases/(${version})/(?:SHA256SUMS|install\\.sh|release\\.json|(?:qinpanel|qonpanel|nusa-panel)-(${version})-(?:x86_64|aarch64)-unknown-linux-gnu\\.tar\\.gz)$`);
    const match = name.match(release);
    if (url.search || !(['install.sh', 'latest.json'].includes(name) || match && (!match[2] || match[1] === match[2]))) {
      return new Response('Not found', { status: 404 });
    }
    const expected = `https://${project}.supabase.co/storage/v1/object/public/`;
    const base = env.SUPABASE_PUBLIC_BASE_URL || expected + 'qonpanel-releases';
    if (!new RegExp(`^https://${project}\\.supabase\\.co/storage/v1/object/public/[a-z0-9][a-z0-9_-]{0,62}$`).test(base)) {
      return new Response('Distribution configuration unavailable', { status: 503 });
    }
    const headers = new Headers();
    for (const header of ['range', 'if-none-match', 'if-modified-since']) {
      if (request.headers.has(header)) headers.set(header, request.headers.get(header));
    }
    headers.set('Accept-Encoding', 'identity');
    const mutable = ['install.sh', 'latest.json'].includes(name);
    // Disabling this Worker's cache does not invalidate Supabase's own CDN.
    // Generate the nonce internally; client queries remain forbidden above.
    const upstreamUrl = new URL(`${base}/${name}`);
    if (mutable) {
      upstreamUrl.searchParams.set('cacheNonce', crypto.randomUUID());
      headers.set('Cache-Control', 'no-cache');
    }
    try {
      const upstream = await fetch(upstreamUrl.href, {
        // workerd supports manual/follow; reject redirect status codes below.
        method: request.method, headers, redirect: 'manual',
        // Missing objects must not stay negatively cached after a new upload.
        // Partial responses must not populate the full-file cache either.
        cf: { cacheTtlByStatus: mutable ? { '100-599': -1 } : { 200: 86400, '201-599': -1 }, cacheEverything: !mutable },
      });
      if (upstream.status === 400) {
        // A HEAD response has no error body. Confirm with one bounded GET to the
        // same allowlisted object, without range/conditional/client credentials.
        let probe = upstream;
        if (request.method === 'HEAD') {
          await upstream.body?.cancel();
          probe = await fetch(upstreamUrl.href, {
            method: 'GET', redirect: 'manual',
            headers: { 'Accept-Encoding': 'identity', 'Cache-Control': 'no-cache' },
            cf: { cacheTtlByStatus: { '100-599': -1 }, cacheEverything: false },
          });
        }
        if (await storageObjectMissing(probe)) {
          return new Response(request.method === 'HEAD' ? null : 'Not found', {
            status: 404, headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
          });
        }
        return new Response(request.method === 'HEAD' ? null : 'Artifact temporarily unavailable', { status: 502 });
      }
      if (![200, 206, 304, 404, 416].includes(upstream.status)) {
        await upstream.body?.cancel();
        return new Response('Artifact temporarily unavailable', { status: 502 });
      }
      const output = new Headers();
      for (const name of ['content-type', 'content-length', 'content-range', 'accept-ranges', 'etag', 'last-modified']) {
        if (upstream.headers.has(name)) output.set(name, upstream.headers.get(name));
      }
      output.set('Cache-Control', upstream.status === 404 || mutable ? 'no-store' : 'public, max-age=86400, immutable');
      output.set('X-Content-Type-Options', 'nosniff');
      return new Response(request.method === 'HEAD' ? null : upstream.body, { status: upstream.status, headers: output });
    } catch {
      return new Response('Artifact temporarily unavailable', { status: 502 });
    }
  },
};
