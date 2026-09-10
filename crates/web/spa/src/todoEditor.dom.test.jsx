// @vitest-environment jsdom
// TodoEditor DOM smoke：三态外壳（表单 / 画布 / JSON 源码）的 switchMode 时序 ——
// spec 是唯一草稿事实来源：离开表单容忍半填先并入 spec（低频字段 metadata /
// required_tool_calls 按 todo id 透传）、离开 JSON 解析失败停留原模式、画布
// （todo/editor/canvasEditor.jsx，React Flow 不加 mock 直挂，同
// editor.dom.test.jsx）节点数 = spec todos 数、切回表单值不丢。
// TodoEditorSession 挂载后自行走网络加载（context/envs/env 三个 GET），所以
// 接缝是 api.js 的模块级 mock（同 todoPanel 模式）；保存断言 PUT context.json
// 且 env 绑定未变时不发第二个 PUT。

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const { apiGetMock, apiPostMock, apiPutMock, apiDelMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
  apiPutMock: vi.fn(),
  apiDelMock: vi.fn(),
}));
vi.mock('./api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPut: apiPutMock,
  apiDel: apiDelMock,
}));

import './test/setup-dom.js';
import { TodoEditor } from './todoEditor.jsx';

/// antd 6 Button 对两字中文自动插空格（「保 存」），按 role + 去空白匹配。
const findButton = (txt) => screen.getAllByRole('button')
  .find((b) => (b.textContent || '').replace(/\s+/g, '') === txt);

const CTX_PATH = '/api/todo/templates/demo/v1/context.json';
const ENV_PATH = '/api/todo/templates/demo/v1/env.json';

/// 裸 WorkflowSpec（specFromContext 接受裸形态）：metadata 非空、t1 带数组形
/// required_tool_calls —— 二者是表单模式不覆盖、formToSpec 需透传的低频字段。
const CONTEXT_FIXTURE = {
  schema_version: 1,
  id: 'wf-demo',
  name: 'demo',
  objective: 'ship the demo',
  constraints: ['不得修改 crates/core'],
  metadata: { owner: 'demo' },
  todos: [
    {
      id: 't1',
      title: '调研方案',
      agent: 'explore',
      depends_on: [],
      max_attempts: 3,
      requirement_background: '背景',
      instructions: '调研',
      acceptance: { criteria: '输出对比文档', required_tool_calls: [{ name: 'web_search', arguments_contains: [] }] },
    },
    {
      id: 't2',
      title: '落地实现',
      agent: 'build',
      depends_on: ['t1'],
      max_attempts: 2,
      requirement_background: '背景2',
      instructions: '实现',
      acceptance: { criteria: '测试通过' },
    },
  ],
};
const ENVS_FIXTURE = { envs: [{ name: 'prod' }] };
const ENV_FIXTURE = { env: '' }; // 未绑定 → 保存时不发 env PUT

const installApi = () => {
  apiGetMock.mockReset().mockImplementation((path) => {
    if (path === CTX_PATH) {
      return Promise.resolve(CONTEXT_FIXTURE);
    }
    if (path === '/api/todo/envs') {
      return Promise.resolve(ENVS_FIXTURE);
    }
    if (path === ENV_PATH) {
      return Promise.resolve(ENV_FIXTURE);
    }
    return Promise.resolve({});
  });
  apiPostMock.mockReset().mockResolvedValue({ ok: true });
  apiPutMock.mockReset().mockResolvedValue({ ok: true });
  apiDelMock.mockReset().mockResolvedValue({ ok: true });
};

beforeEach(installApi);

afterEach(() => {
  cleanup();
});

/// 渲染外壳并等首个表单字段回填（= context 加载完成的信号）。
const mountEditor = async () => {
  render(<TodoEditor templateName="demo" version="v1" onNotice={() => {}} onClose={vi.fn()} />);
  return screen.findByDisplayValue('ship the demo');
};

describe('TodoEditor 三态外壳', () => {
  it('加载后落在表单模式并回填 spec 高频字段', async () => {
    await mountEditor();
    expect(apiGetMock).toHaveBeenCalledWith(CTX_PATH);
    expect(apiGetMock).toHaveBeenCalledWith('/api/todo/envs');
    expect(apiGetMock).toHaveBeenCalledWith(ENV_PATH);
    expect(screen.getByDisplayValue('demo')).toBeTruthy(); // 名称
    expect(screen.getByDisplayValue('调研方案')).toBeTruthy(); // todos[0].title
    expect(screen.getByText('TODO 列表')).toBeTruthy(); // 表单模式本体
    expect(screen.queryByLabelText('spec-json')).toBeNull(); // JSON 视图未挂
  });

  it('表单改目标后切「JSON 源码」：新目标并入且 metadata/required_tool_calls 透传', async () => {
    await mountEditor();
    fireEvent.change(screen.getByDisplayValue('ship the demo'), { target: { value: 'ship the demo v2' } });
    fireEvent.click(screen.getByText('JSON 源码'));
    const area = screen.getByLabelText('spec-json');
    const parsed = JSON.parse(area.value);
    expect(parsed.objective).toBe('ship the demo v2'); // 表单编辑已并入 spec
    expect(parsed.schema_version).toBe(1);
    expect(parsed.metadata).toEqual({ owner: 'demo' }); // original 低频字段保留
    const t1 = parsed.todos.find((t) => t.id === 't1');
    expect(t1.acceptance.required_tool_calls[0].name).toBe('web_search'); // 按 id 透传
  });

  it('JSON 非法时切回表单被阻止：停留 JSON 模式并提示解析失败', async () => {
    await mountEditor();
    fireEvent.click(screen.getByText('JSON 源码'));
    const area = screen.getByLabelText('spec-json');
    fireEvent.change(area, { target: { value: '{' } });
    fireEvent.click(screen.getByText('表单'));
    expect(screen.getByLabelText('spec-json')).toBeTruthy(); // 模式未切换
    expect(await screen.findByText(/JSON 解析失败/)).toBeTruthy();
  });

  it('JSON 改名后切回表单：解析后的 spec 回灌表单', async () => {
    await mountEditor();
    fireEvent.click(screen.getByText('JSON 源码'));
    const area = screen.getByLabelText('spec-json');
    fireEvent.change(area, { target: { value: JSON.stringify({ ...CONTEXT_FIXTURE, name: 'demo-renamed' }) } });
    fireEvent.click(screen.getByText('表单'));
    expect(await screen.findByDisplayValue('demo-renamed')).toBeTruthy();
    expect(screen.getByDisplayValue('ship the demo')).toBeTruthy(); // 其余字段同回灌
  });

  it('画布模式渲染 spec 数量的节点，切回表单编辑不丢', async () => {
    await mountEditor();
    fireEvent.change(screen.getByDisplayValue('ship the demo'), { target: { value: 'ship the demo v2' } });
    fireEvent.click(screen.getByText('画布'));
    await waitFor(() => expect(document.querySelectorAll('.dag-edit-node')).toHaveLength(2));
    fireEvent.click(screen.getByText('表单'));
    expect(await screen.findByDisplayValue('ship the demo v2')).toBeTruthy(); // 表单编辑经 spec 往返
    expect(screen.getByDisplayValue('demo')).toBeTruthy();
  });

  it('表单模式保存：合并 spec PUT 到 context.json，env 未变不重发', async () => {
    await mountEditor();
    fireEvent.change(screen.getByDisplayValue('ship the demo'), { target: { value: 'ship the demo v3' } });
    fireEvent.click(findButton('保存'));
    await waitFor(() => expect(apiPutMock).toHaveBeenCalledTimes(1));
    expect(apiPutMock.mock.calls[0][0]).toBe(CTX_PATH);
    const body = apiPutMock.mock.calls[0][1];
    expect(body.objective).toBe('ship the demo v3'); // 表单编辑并入
    expect(body.metadata).toEqual({ owner: 'demo' }); // 低频字段随 PUT 上行
    expect(body.todos.find((t) => t.id === 't1').acceptance.required_tool_calls[0].name).toBe('web_search');
    expect(apiPutMock).not.toHaveBeenCalledWith(ENV_PATH, expect.anything()); // env 绑定未变
  });
});
