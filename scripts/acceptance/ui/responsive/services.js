const { createServer } = require('node:http');
const { readFile } = require('node:fs/promises');
const { accessSync, constants, existsSync } = require('node:fs');
const { execFileSync, spawnSync } = require('node:child_process');
const path = require('node:path');
const { FIXTURES, ABSENT, GUARDED } = require('../../spa_responsive_fixtures');
module.exports = function services(REPO, TOKEN) {
const SPA = path.join(REPO, 'crates/web/spa/dist');
const DRIFT_SCRIPT = path.join(REPO, 'scripts/check-spa-drift.sh');
const fixtureMisses = [];
const porcelain = (...pathspecs) => {
  try {
    return execFileSync('git', ['-C', REPO, 'status', '--porcelain', '--', ...pathspecs],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
  } catch (_) {
    return null; // no git binary, not a repository, or git refused
  }
};

const changedPaths = (status) => status.split('\n').filter((l) => l.trim()).length;

// -> { line, warn, pinned, refusal }. `line` is always printed; `pinned` is
// true only when the bundle on disk is exactly HEAD and src/ is clean.
function bundleProvenance() {
  const dist = porcelain('crates/web/spa/dist');
  const src = porcelain('crates/web/spa/src');
  if (dist === null || src === null) {
    return {
      line: `bundle: ${SPA} (git unavailable: provenance unknown)`,
      warn: null,
      pinned: false,
      refusal: 'git is unavailable here, so the bundle cannot be tied to a commit',
    };
  }
  const distCount = changedPaths(dist);
  const srcCount = changedPaths(src);
  if (!distCount && !srcCount) {
    return {
      line: `bundle: ${SPA} (clean: dist == HEAD, no uncommitted src)`,
      warn: null,
      pinned: true,
      refusal: '',
    };
  }
  return {
    line: `bundle: ${SPA} (working tree; dist differs from HEAD: ${distCount} path(s), `
      + `src differs from HEAD: ${srcCount} path(s))`,
    // src moved but dist did not: the bundle about to be measured was built
    // from an older src/, so a green run says nothing about the edits on disk.
    warn: srcCount && !distCount
      ? `WARNING: crates/web/spa/src has ${srcCount} uncommitted change(s) while dist/ matches HEAD`
        + ' -- the bundle being measured does NOT contain them (run scripts/build-spa.sh)'
      : null,
    pinned: false,
    refusal: `the working tree differs from HEAD (dist: ${distCount} path(s), src: ${srcCount} path(s))`,
  };
}

// --drift: rebuild src/ into a temp dir and diff it against dist/ before
// measuring, so "dist/ is what src/ says" is verified instead of assumed.
function runDriftCheck() {
  if (!existsSync(DRIFT_SCRIPT)) {
    console.log(`drift: skipped, ${DRIFT_SCRIPT} does not exist`);
    return;
  }
  try {
    accessSync(DRIFT_SCRIPT, constants.X_OK);
  } catch (_) {
    console.log(`drift: skipped, ${DRIFT_SCRIPT} is not executable`);
    return;
  }
  console.log(`drift: ${DRIFT_SCRIPT} (rebuilds the SPA into a temp dir -- slow)`);
  const run = spawnSync(DRIFT_SCRIPT, [], { cwd: REPO, stdio: 'inherit' });
  if (run.error) {
    console.error(`drift: could not run ${DRIFT_SCRIPT}: ${run.error.message}`);
    process.exit(2);
  }
  if (run.status !== 0) {
    console.error(`drift: FAIL (exit ${run.status}) -- dist/ is not a build of the current src/;`
      + ' run scripts/build-spa.sh and commit dist/');
    process.exit(2);
  }
  console.log('drift: OK, dist/ matches a fresh build of src/');
}

// ---------------------------------------------------------------------------
// Static + fixture server. Only GETs are served (every page is read-only); a
// non-GET proves the gate clicked a mutating control and is answered 405.
// ---------------------------------------------------------------------------
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png',
  '.json': 'application/json; charset=utf-8', '.map': 'application/json' };

async function serve(port) {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, `http://127.0.0.1:${port}`);
    const send = (code, body, type) => {
      res.writeHead(code, { 'content-type': type || 'application/json; charset=utf-8', 'cache-control': 'no-store' });
      res.end(body);
    };
    if (url.pathname.startsWith('/api/')) {
      if (req.method !== 'GET') {
        fixtureMisses.push(`${req.method} ${url.pathname}`);
        return send(405, JSON.stringify({ error: 'gate is read-only' }));
      }
      if (GUARDED.some((g) => url.pathname.startsWith(g)) && (req.headers.authorization || '') !== `Bearer ${TOKEN}`) {
        return send(401, JSON.stringify({ error: 'unauthorized' }));
      }
      const pathname = decodeURIComponent(url.pathname);
      const key = `${pathname}${url.search}`;
      if (ABSENT.includes(key) || ABSENT.includes(url.pathname)) {
        return send(404, JSON.stringify({ error: 'not found' }));
      }
      if (FIXTURES[pathname]) return send(200, JSON.stringify(FIXTURES[pathname]));
      fixtureMisses.push(`GET ${key}`);
      console.log(`  fixture miss: GET ${key}`);
      return send(404, JSON.stringify({ error: 'no fixture' }));
    }
    // SPA fallback: unknown paths serve index.html so client routes load.
    const rel = url.pathname === '/' ? 'index.html' : url.pathname.replace(/^\/+/, '');
    const file = path.join(SPA, rel);
    if (!file.startsWith(SPA) || !existsSync(file)) {
      return send(200, await readFile(path.join(SPA, 'index.html')), TYPES['.html']);
    }
    try {
      return send(200, await readFile(file), TYPES[path.extname(file)] || 'application/octet-stream');
    } catch (_) { return send(404, JSON.stringify({ error: 'missing asset' })); }
  });
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  return { fixtureMisses, close: () => new Promise((r) => server.close(r)) };
}

// ---------------------------------------------------------------------------
// In-page measurement.
// ---------------------------------------------------------------------------

return { serve, bundleProvenance, runDriftCheck };
};
