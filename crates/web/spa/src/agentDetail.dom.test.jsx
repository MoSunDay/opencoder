// @vitest-environment jsdom
// AgentDetail DOM smoke：卡片渲染 + Meta 历史；引用 Select 换选命中 PUT
// /api/agents/:name（整卡 current）；prompt 编辑器预填 CURRENT 版本三文件
// （b64 解码）且「保存」命中 PUT /api/agents/resources/prompts/:name ——
// 断言解码后的三份文件内容；版本 Select + 回滚命中 POST rollback。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPutMock, apiPostMock, apiPatchMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPatchMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPut: apiPutMock,
  apiPost: apiPostMock,
  apiPatch: apiPatchMock,
}));

import './test/setup-dom.js';
import { b64DecodeText, b64EncodeText } from './agentsItems.js';
import { AgentDetail } from './agentDetail.jsx';

const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const pickSelectOption = async (selectEl, label) => {
  await act(async () => {
    fireEvent.mouseDown(selectEl);
  });
  const option = await waitFor(() => {
    const all = [...document.querySelectorAll('.ant-select-item-option')];
    const hit = all.find((o) => o.getAttribute('title') === label || o.textContent === label);
    expect(hit).toBeTruthy();
    return hit;
  });
  await act(async () => {
    fireEvent.click(option);
  });
};

const SOUL = '你是严谨的 Rust 工程师';
const HOW = '先读再改，测试兜底';
const OUTPUT = '只输出 JSON';
const metaFixture = {
  meta: {
    name: 'coder',
    created_at: '2026-09-01T00:00:00Z',
    updated_at: '2026-09-02T00:00:00Z',
    current: { prompt: 'base', skills: null, tools: 'std', memory: null },
    history: [
      { at: '2026-09-02T00:00:00Z', field: 'prompt', from: null, to: 'base' },
      { at: '2026-09-01T00:00:00Z', field: 'tools', from: null, to: 'std' },
    ],
    references: { prompt_files: ['soul', 'how', 'output'], skills: [], tools: ['bash'], memory: false },
  },
};
const promptsMetaFixture = { meta: { name: 'base', current: 2, history: [1, 2] } };
const resourcesFixture = {
  prompts: [
    { name: 'base', current: 2, versions: [1, 2] },
    { name: 'alt', current: 1, versions: [1] },
  ],
  skills: [],
  tools: [{ name: 'std', current: 1, versions: [1] }],
  memory: [],
};

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === '/api/agents/coder/meta') {
      return Promise.resolve(metaFixture);
    }
    if (path === '/api/agents/resources/prompts/base/meta') {
      return Promise.resolve(promptsMetaFixture);
    }
    if (path === '/api/agents/resources/prompts/base/versions/2/files/soul.md') {
      return Promise.resolve({ ok: true, path: 'soul.md', content_b64: b64EncodeText(SOUL), size: 9 });
    }
    if (path === '/api/agents/resources/prompts/base/versions/2/files/how.md') {
      return Promise.resolve({ ok: true, path: 'how.md', content_b64: b64EncodeText(HOW), size: 9 });
    }
    if (path === '/api/agents/resources/prompts/base/versions/2/files/output.md') {
      return Promise.resolve({ ok: true, path: 'output.md', content_b64: b64EncodeText(OUTPUT), size: 9 });
    }
    return Promise.resolve({ ok: true });
  });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiPostMock.mockReset().mockResolvedValue({ ok: true, version: 3, current: 1 });
  apiPatchMock.mockReset().mockResolvedValue({ ok: true, active: 'coder' });
};

beforeEach(() => {
  installApi();
});

afterEach(() => {
  cleanup();
});

const mountDetail = async () => {
  render(
    <AgentDetail
      name="coder"
      resources={resourcesFixture}
      onNotice={() => {}}
      onChanged={() => {}}
      onBack={() => {}}
    />,
  );
  expect(await screen.findByText('Agent: coder')).toBeTruthy();
};

describe('AgentDetail', () => {
  it('renders the card, resolved snapshot tags and the Meta history timeline', async () => {
    await mountDetail();
    // prompt tab 默认激活：引用 Select 显示 base（上屏的是 option label
    // `base · v2`），解析快照给出文件主干。
    expect(await screen.findByText('base · v2')).toBeTruthy();
    ['soul', 'how', 'output'].forEach((n) => {
      expect(screen.getAllByText(n).length).toBeGreaterThanOrEqual(1);
    });
    // Meta tab：两条历史按 field/from→to 渲染（field 名在快照卡里也有，
    // 用 findAllByText）。
    fireEvent.click(screen.getByText('Meta'));
    expect(await screen.findByText('引用变更历史')).toBeTruthy();
    expect(screen.getByText('— → base')).toBeTruthy();
    expect(screen.getByText('— → std')).toBeTruthy();
    expect((await screen.findAllByText('prompt')).length).toBeGreaterThanOrEqual(2);
  });

  it('fires PUT /api/agents/:name with the whole card when the ref select changes', async () => {
    await mountDetail();
    await pickSelectOption(document.querySelector('[aria-label="ref-select-prompt"]'), 'alt · v1');
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith('/api/agents/coder', {
        current: { prompt: 'alt', skills: null, tools: 'std', memory: null },
      });
    });
  });

  it('rolls back through POST rollback with the picked version', async () => {
    await mountDetail();
    await pickSelectOption(document.querySelector('[aria-label="rollback-version-prompt"]'), 'v1');
    fireEvent.click(findButton('回滚'));
    await waitFor(() => {
      expect(apiPostMock).toHaveBeenCalledWith('/api/agents/resources/prompts/base/rollback', { version: 1 });
    });
  });

  it('prefills the three prompt files decoded from b64, then saves them back', async () => {
    await mountDetail();
    const soul = await waitFor(() => {
      const el = document.querySelector('[aria-label="prompt-soul"]');
      expect(el).toBeTruthy();
      expect(el.value).toBe(SOUL); // b64 → UTF-8 解码预填
      return el;
    });
    expect(document.querySelector('[aria-label="prompt-how"]').value).toBe(HOW);
    expect(document.querySelector('[aria-label="prompt-output"]').value).toBe(OUTPUT);
    fireEvent.change(soul, { target: { value: SOUL + ' v2' } });
    fireEvent.click(findButton('保存'));
    await waitFor(() => {
      expect(apiPutMock).toHaveBeenCalledWith('/api/agents/resources/prompts/base', {
        files: [
          { path: 'soul.md', content_b64: b64EncodeText(SOUL + ' v2') },
          { path: 'how.md', content_b64: b64EncodeText(HOW) },
          { path: 'output.md', content_b64: b64EncodeText(OUTPUT) },
        ],
      });
    });
    // 断言解码后的内容（不是裸 b64 串；atob 只给字节串，解码走同一助手）。
    const body = apiPutMock.mock.calls.find((c) => c[0] === '/api/agents/resources/prompts/base')[1];
    expect(b64DecodeText(body.files[1].content_b64)).toBe(HOW);
    expect(b64DecodeText(body.files[2].content_b64)).toBe(OUTPUT);
  });

  it('activates the agent through PATCH /api/agents/active', async () => {
    await mountDetail();
    fireEvent.click(findButton('设为生效'));
    await waitFor(() => {
      expect(apiPatchMock).toHaveBeenCalledWith('/api/agents/active', { active: 'coder' });
    });
  });
});
