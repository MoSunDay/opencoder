// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { clearCredentials, setCredentials, setState } from '../store.js';
import { artifactPath, downloadArtifact } from './download.js';

class TestChannel {
  constructor() {
    this.port1 = {};
    this.port2 = {
      postMessage: (data) => this.port1.onmessage?.({ data }),
    };
  }
}

afterEach(() => {
  clearCredentials();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('bounded artifact downloads', () => {
  it('allows an anonymous download only after authentication is disabled', async () => {
    clearCredentials();
    await expect(downloadArtifact('dag-run', 'build', 'output.txt')).rejects.toThrow('请先登录');
    setState({ authEnabled: false });
    setState({ base: '/service', token: 'unused-token' });
    Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: undefined });
    const append = vi.spyOn(document.body, 'appendChild').mockImplementation((element) => element);
    await downloadArtifact('dag-run', 'build', 'output.txt', 0);
    const frame = append.mock.calls[0][0];
    expect(frame.getAttribute('src')).toBe('/service/api/executions/dag-run/artifact?step=build&file=output.txt&index=0');
    expect(frame.getAttribute('src')).not.toContain('unused-token');
  });
  it.each(['', '/service'])('registers a bearer download without buffering at %s', async (base) => {
    setCredentials('secret-token', base);
    const posted = [];
    const active = {
      scriptURL: new URL(`${base}/static/download-sw.js`, window.location.href).href,
      postMessage: (record, ports) => {
        posted.push(record);
        ports[0].postMessage({ ok: true });
      },
    };
    Object.defineProperty(navigator, 'serviceWorker', {
      configurable: true,
      value: {
        controller: active,
        register: vi.fn().mockResolvedValue({ active }),
        ready: Promise.resolve({ active }),
      },
    });
    vi.stubGlobal('MessageChannel', TestChannel);
    vi.stubGlobal('crypto', { randomUUID: () => 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee' });
    vi.stubGlobal('Blob', vi.fn());
    const replace = vi.fn();
    vi.spyOn(window, 'open').mockReturnValue({ location: { replace } });

    await downloadArtifact('dag-run', 'build', 'output.txt');

    expect(posted).toEqual([{
      type: 'opencoder-download',
      id: 'aaaaaaaabbbbccccddddeeeeeeeeeeee',
      path: `${base}/api/executions/dag-run/artifact?step=build&file=output.txt`,
      token: 'secret-token',
      clientUrl: window.location.href.split('#')[0],
    }]);
    expect(replace).toHaveBeenCalledWith(`${base}/__opencoder_download/aaaaaaaabbbbccccddddeeeeeeeeeeee`);
    expect(globalThis.Blob).not.toHaveBeenCalled();
  });

  it('only constructs same-origin artifact endpoint paths', () => {
    expect(artifactPath('dag-a', 'step/x', 'output.json'))
      .toBe('/api/executions/dag-a/artifact?step=step%2Fx&file=output.json');
  });

  it('addresses a dynamic instance without dropping its evidence path', () => {
    expect(artifactPath('dag-a', 'cases', 'evidence/raw.json', 17))
      .toBe('/api/executions/dag-a/artifact?step=cases&file=evidence%2Fraw.json&index=17');
    for (const index of [-1, 1.5, NaN]) expect(() => artifactPath('a', 'cases', 'raw', index)).toThrow();
  });

  it('limits a prefixed worker to artifact paths inside its own origin and scope', () => {
    const source = readFileSync('public/static/download-sw.js', 'utf8');
    const valid = runInNewContext(`${source}; validArtifactPath`, { URL, Map, self: {
      registration: { scope: 'https://example.test/service/' },
      addEventListener: () => {},
    } });
    const artifact = '/api/executions/dag-a/artifact?step=write&file=report.txt';
    expect(valid('/service' + artifact, 'https://example.test')).toBe(true);
    for (const path of [artifact, '/another' + artifact, 'https://other.test/service' + artifact,
      '/service/api/executions/dag-a', '/service/../api/executions/dag-a/artifact?step=write&file=report.txt']) {
      expect(valid(path, 'https://example.test')).toBe(false);
    }
  });

  it('binds navigation to the current page after its URL is changed by the app', async () => {
    const handlers = {}, acknowledgements = [];
    const sourceUrl = 'https://example.test/service/?base=%2Fservice';
    const currentUrl = 'https://example.test/service/?brain_run=one';
    const fetchFile = vi.fn(async () => ({ status: 200 }));
    runInNewContext(readFileSync('public/static/download-sw.js', 'utf8'), {
      URL, Map, Date, setTimeout: vi.fn(), fetch: fetchFile,
      Response: class { constructor(body, options) { this.status = options.status; } },
      self: { registration: { scope: 'https://example.test/service/' },
        location: { origin: 'https://example.test' },
        clients: { get: async () => ({ url: sourceUrl }) },
        addEventListener: (name, handler) => { handlers[name] = handler; } },
    });
    const register = (id, clientUrl = currentUrl) => handlers.message({
      source: { id: 'owner', url: sourceUrl }, ports: [{ postMessage: (value) => acknowledgements.push(value) }],
      data: { type: 'opencoder-download', id, token: 'fixture', clientUrl,
        path: '/service/api/executions/dag-a/artifact?step=write&file=report.txt' },
    });
    const request = (id, mode, referrer) => new Promise((resolve) => handlers.fetch({ clientId: '',
      request: { url: `https://example.test/service/__opencoder_download/${id}`, mode, referrer },
      respondWith: (result) => Promise.resolve(result).then(resolve),
    }));
    register('valid');
    expect((await request('valid', 'navigate', currentUrl)).status).toBe(200);
    expect(fetchFile).toHaveBeenCalledOnce();
    register('wrong-page');
    expect((await request('wrong-page', 'navigate', 'https://example.test/service/other')).status).toBe(410);
    register('wrong-mode');
    expect((await request('wrong-mode', 'cors', currentUrl)).status).toBe(410);
    register('wrong-origin', 'https://other.test/service/');
    expect(acknowledgements.at(-1).ok).toBe(false);
    expect(fetchFile).toHaveBeenCalledOnce();
  });

  it('keeps credentials ephemeral and rejects redirected or cross-origin upstreams', () => {
    const source = readFileSync('public/static/download-sw.js', 'utf8');
    expect(source).toContain("url.origin === origin");
    expect(source).toContain("self.clients.get(record.clientId)");
    expect(source).toContain("event.clientId === record.clientId");
    expect(source).toContain("pending.delete(match[1])");
    expect(source).toContain("redirect: 'error'");
    expect(source).not.toContain('caches.');
    expect(source).not.toContain('localStorage');
  });
});
