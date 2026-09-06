// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { clearCredentials, setCredentials } from '../store.js';
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
  it('registers one bearer request and clicks an opaque URL without buffering', async () => {
    setCredentials('secret-token', '');
    const posted = [];
    const active = {
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
      path: '/api/executions/dag-run/artifact?step=build&file=output.txt',
      token: 'secret-token',
    }]);
    expect(replace).toHaveBeenCalledWith('/__opencoder_download/aaaaaaaabbbbccccddddeeeeeeeeeeee');
    expect(globalThis.Blob).not.toHaveBeenCalled();
  });

  it('only constructs same-origin artifact endpoint paths', () => {
    expect(artifactPath('dag-a', 'step/x', 'output.json'))
      .toBe('/api/executions/dag-a/artifact?step=step%2Fx&file=output.json');
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
