const pending = new Map();
const TTL_MS = 60_000;
const base = new URL(self.registration.scope).pathname.replace(/\/$/, '');
const ARTIFACT_PATH = /^\/api\/executions\/[A-Za-z0-9_-]+\/artifact$/;

self.addEventListener('install', (event) => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

function validArtifactPath(raw, origin) {
  try {
    const url = new URL(raw, origin);
    return url.origin === origin && url.pathname.startsWith(`${base}/`) && ARTIFACT_PATH.test(url.pathname.slice(base.length))
      && url.searchParams.has('step') && url.searchParams.has('file');
  } catch {
    return false;
  }
}

function validClientUrl(raw, origin) {
  try {
    const url = new URL(raw);
    return url.origin === origin && url.pathname.startsWith(`${base}/`) && !url.hash;
  } catch { return false; }
}

self.addEventListener('message', (event) => {
  const data = event.data || {};
  const source = event.source;
  const clientUrl = data.clientUrl || source?.url?.split('#')[0];
  if (data.type !== 'opencoder-download' || !source || !data.id
      || typeof data.token !== 'string' || !validArtifactPath(data.path, self.location.origin)
      || !validClientUrl(clientUrl, self.location.origin)) {
    event.ports[0]?.postMessage({ ok: false, error: 'invalid download registration' });
    return;
  }
  const record = {
    clientId: source.id,
    // Client.url can retain the original navigation URL after history.replaceState.
    sourceUrl: source.url,
    clientUrl,
    path: data.path,
    token: data.token,
    expires: Date.now() + TTL_MS,
  };
  pending.set(data.id, record);
  setTimeout(() => pending.delete(data.id), TTL_MS);
  event.ports[0]?.postMessage({ ok: true });
});

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);
  if (!url.pathname.startsWith(`${base}/`)) return;
  const match = url.pathname.slice(base.length).match(/^\/__opencoder_download\/([A-Za-z0-9_-]+)$/);
  if (!match) return;
  const record = pending.get(match[1]);
  pending.delete(match[1]);
  if (!record || record.expires < Date.now()) {
    event.respondWith(new Response('download registration expired', { status: 410 }));
    return;
  }
  event.respondWith((async () => {
    const owner = await self.clients.get(record.clientId);
    const directClient = event.clientId === record.clientId;
    const delegatedNavigation = owner && owner.url === record.sourceUrl
      && event.request.mode === 'navigate' && event.request.referrer === record.clientUrl;
    if (!directClient && !delegatedNavigation) {
      return new Response('download client mismatch', { status: 410 });
    }
    return fetch(record.path, {
      method: 'GET',
      headers: record.token ? { Authorization: `Bearer ${record.token}` } : {},
      cache: 'no-store',
      credentials: 'same-origin',
      redirect: 'error',
    });
  })());
});
