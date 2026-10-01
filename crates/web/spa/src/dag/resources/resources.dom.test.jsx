// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import '../../test/setup-dom.js';

const mocks = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn(), apiDel: vi.fn(), authFetch: vi.fn() }));
vi.mock('../../api.js', () => mocks);
import { BinaryResources } from './panel.jsx';
import { BinaryEditor } from './editor.jsx';
import { BinaryResourceField } from './field.jsx';

const pool = { name: 'tool', description: 'compiler', current: 2, current_version: { size_bytes: 120, sha256: 'a'.repeat(64) }, history: [{ version: 1, sha256: 'b'.repeat(64), size_bytes: 120 }, { version: 2, sha256: 'a'.repeat(64), size_bytes: 120 }] };
beforeEach(() => {
  Object.values(mocks).forEach((mock) => mock.mockReset());
  mocks.apiGet.mockImplementation(async (path) => path === '/api/dag/binaries' ? { pools: [pool] } : pool);
  mocks.apiPost.mockResolvedValue({ ok: true }); mocks.apiPut.mockResolvedValue({ ok: true }); mocks.apiDel.mockResolvedValue({ ok: true });
});

describe('binary resource UI', () => {
  it('shows actual versions and confirms pointer-only switches', async () => {
    render(<BinaryResources />);
    fireEvent.click(await screen.findByRole('button', { name: 'tool' }));
    fireEvent.click(await screen.findByRole('button', { name: '使用 v1' }));
    expect(mocks.apiPost).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole('button', { name: '确认切换版本' }));
    await waitFor(() => expect(mocks.apiPost).toHaveBeenCalledWith('/api/dag/binaries/tool/rollback', { version: 1 }));
  });
  it('requires confirmation before deleting all versions', async () => {
    render(<BinaryResources />);
    fireEvent.click(await screen.findByRole('button', { name: '删除资源' }));
    expect(mocks.apiDel).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole('button', { name: '确认删除资源' }));
    await waitFor(() => expect(mocks.apiDel).toHaveBeenCalledWith('/api/dag/binaries/tool'));
  });
  it('exposes list failures rather than reporting an empty pool', async () => {
    mocks.apiGet.mockRejectedValueOnce(new Error('pool unavailable'));
    render(<BinaryResources />);
    expect(await screen.findByText('pool unavailable')).toBeTruthy();
    expect(screen.queryByText('暂无二进制资源')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '重试资源列表' }));
    expect(await screen.findByRole('button', { name: 'tool' })).toBeTruthy();
  });
  it('keeps upload input and error visible when save fails', async () => {
    mocks.apiPost.mockRejectedValueOnce(new Error('conflicting resource'));
    const onSaved = vi.fn();
    render(<BinaryEditor onClose={vi.fn()} onSaved={onSaved} />);
    fireEvent.change(screen.getByLabelText('二进制资源名称'), { target: { value: 'new-tool' } });
    const bytes = new Uint8Array(120); bytes.set([127, 69, 76, 70, 2, 1, 1]); bytes[16] = 2; bytes[18] = 62;
    const file = new File([bytes], 'new-tool'); file.arrayBuffer = async () => bytes.buffer;
    fireEvent.change(screen.getByLabelText('Linux 可执行文件'), { target: { files: [file] } });
    fireEvent.click(screen.getByRole('button', { name: '保存二进制' }));
    expect(await screen.findByText('conflicting resource')).toBeTruthy();
    expect(screen.getByLabelText('二进制资源名称').value).toBe('new-tool');
    expect(onSaved).not.toHaveBeenCalled();
    expect(mocks.apiPost.mock.calls[0][1].binary_b64).toBe(btoa(String.fromCharCode(...bytes)));
  });
  it('preserves the selected token and disables selection on resource failures', async () => {
    mocks.apiGet.mockRejectedValue(new Error('node offline'));
    const onChange = vi.fn();
    render(<BinaryResourceField value="tool@v2" onChange={onChange} />);
    expect(await screen.findByText('资源读取失败：node offline')).toBeTruthy();
    expect(screen.getByRole('combobox', { name: '二进制资源' }).closest('.ant-select').className).toContain('ant-select-disabled');
    expect(onChange).not.toHaveBeenCalled();
  });
});
