// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { HarnessManagement } from './management.jsx';
import { RunnerDetail } from '../fleet/detail/runner.jsx';
import { apiGet, apiPut } from '../api.js';
import { downloadArtifact } from '../fleet/download.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPut: vi.fn() }));
vi.mock('../fleet/download.js', () => ({ downloadArtifact: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
it('creates a named Codex profile containing only wrap parameters', async () => {
  apiGet.mockResolvedValue({ harnesses: [{ name: 'codex', settings: { envs: {} } }], profiles: [], agents: [] });
  apiPut.mockResolvedValue({ revision: 1 });
  render(<HarnessManagement onNotice={vi.fn()} />);
  await screen.findByText('尚未保存统一配置');
  fireEvent.change(screen.getByLabelText('new-codex-profile'), { target: { value: 'business' } });
  fireEvent.click(screen.getByRole('button', { name: '新增配置档案' }));
  fireEvent.change(screen.getByLabelText('模型（--model）'), { target: { value: 'model-business' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 Codex 配置' }));
  await waitFor(() => expect(apiPut).toHaveBeenCalledWith('/api/harnesses/codex/profiles/business', { model: 'model-business', envs: {} }));
});
it('shows business verdict separately from execution and downloads the report', async () => {
  render(<RunnerDetail id="dag-business" onNotice={vi.fn()} detail={{ execution: { status: 'done' }, request: { input: { job_id: 'job-business', attempt: 2 } },
    annotations: { delivery_status: 'failed', delivery_error: 'Delivery unavailable' }, runners: [{ step: 'workflow', runner: 'business', agent: 'regression-test',
      verdict: 'block', stage: 'reporting', summary: '接口回归阻断', configuration: { revision: 3, profile: 'business', profile_revision: 4,
        resources: { skills: { name: 'review', version: 'v2' } } }, artifacts: [{ file: 'report.html' }] }] }} />);
  for (const text of ['job-business', '阻断', 'business · v4', 'skills: review v2', '接口回归阻断']) expect(screen.getByText(text)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'report.html' }));
  await waitFor(() => expect(downloadArtifact).toHaveBeenCalledWith('dag-business', 'workflow', 'artifacts/report.html'));
});
