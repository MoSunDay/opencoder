// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { DataValue, RawData, ReadableText } from './value.jsx';
import { ExecutionInputs, ExecutionResult } from './execution.jsx';
import { downloadArtifact } from '../../fleet/download.js';
vi.mock('../../fleet/download.js', () => ({ downloadArtifact: vi.fn() }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
it('保留 false、0、null、空数组和未知字段，对象数组按列展示', () => {
  render(<DataValue value={{ passed: false, count: 0, missing: null, list: [], extra: [{ a: 1, b: '值' }, { a: 2, b: '另一个值' }] }} />);
  for (const text of ['false', '0', 'null', '[]', 'passed', 'extra']) expect(screen.getByText(text)).toBeTruthy();
  expect(screen.getByRole('columnheader', { name: 'a' })).toBeTruthy();
  expect(screen.getByText('另一个值')).toBeTruthy();
});
it('长文可展开，Markdown 清理危险内容，原始 JSON 默认收起', () => {
  const { container } = render(<><ReadableText markdown text={'# 结果\n\n<script>alert(1)</script>\n' + '长说明'.repeat(500)} /><RawData value={{ unknown: '原始值' }} /></>);
  expect(screen.getByRole('heading', { name: '结果' })).toBeTruthy();
  expect(container.querySelector('script')).toBeNull();
  expect(screen.queryByText(/原始值/)).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '展开全文' }));
  expect(screen.getByRole('button', { name: '收起内容' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '查看原始数据' }));
  expect(container.querySelector('pre').textContent).toContain('原始值');
});
it('实际运行输入优先于计划默认值，下载使用原执行及实例 0', async () => {
  downloadArtifact.mockResolvedValue();
  render(<ExecutionInputs id="brain-a" detail={{ request: { input: { layered_request: { inputs: { task: '实际输入' }, plan: { inputs: { task: '计划默认值' } } } } },
    result: { scheduler_artifacts: [{ execution: { id: 'dag-a', kind: 'dag' }, step: 'build', file: 'report.txt', index: 0, bytes: 0, sha256: 'a'.repeat(64) }] } }} />);
  expect(screen.getByText('实际输入')).toBeTruthy();
  expect(screen.queryByText('计划默认值')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '下载文件' }));
  await waitFor(() => expect(downloadArtifact).toHaveBeenCalledWith('dag-a', 'build', 'report.txt', 0));
});
it('执行成功不覆盖结构化结果的失败结论', () => {
  render(<ExecutionResult id="agent-a" detail={{ execution: { status: 'done' }, result: { scheduler_output: { passed: false, reason: '条件未满足' } } }} />);
  expect(screen.getByText('false')).toBeTruthy();
  expect(screen.getByText('条件未满足')).toBeTruthy();
});
