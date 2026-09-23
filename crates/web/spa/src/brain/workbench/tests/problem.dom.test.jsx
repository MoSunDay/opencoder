// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { LayeredRunBody } from '../layered/run.jsx';
import { downloadArtifact } from '../../../fleet/download.js';
vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../../../fleet/download.js', () => ({ downloadArtifact: vi.fn().mockResolvedValue(undefined) }));
vi.mock('../milestone/canvas.jsx', () => ({ MilestoneCanvas: () => <div>milestone canvas</div> }));
afterEach(cleanup);
it('shows original PC problem and downloadable outcomes on the schema 5 body', async () => {
  const view = { schema_version: 5, run: { phase: 'paused', round: 1, max_rounds: 2, activation: 0, layer: 0, valid_layers: 0 },
    plan: { title: 'PC 问题', nodes: [] }, layers: [], events: [], operations: [],
    problem: { text: '生成素材后无法导入时间线', images: [] },
    problem_results: [{ stage: 'conclude', round: 1, execution_id: 'operator-evidence', result: {
      outcome: 'unresolved', summary: '真实证据不足，尚未修复', evidence: [{ path: '/private/receipt.json', sha256: 'a'.repeat(64),
        artifact: { execution: { id: 'operator-evidence' }, step: 'pc-evidence', file: 'receipt.json' } }],
    } }],
  };
  render(<LayeredRunBody view={view} id="brain-pc" refresh={vi.fn()} />);
  expect(screen.getByText('生成素材后无法导入时间线')).toBeTruthy();
  expect(screen.getByText('milestone canvas')).toBeTruthy();
  expect(screen.queryByLabelText('新的轮次预算')).toBeNull();
  fireEvent.click(screen.getByText('未解决'));
  await screen.findByText('真实证据不足，尚未修复');
  fireEvent.click(screen.getByText('receipt.json · 下载证据').closest('button'));
  await waitFor(() => expect(downloadArtifact).toHaveBeenCalledWith('operator-evidence', 'pc-evidence', 'receipt.json'));
});
