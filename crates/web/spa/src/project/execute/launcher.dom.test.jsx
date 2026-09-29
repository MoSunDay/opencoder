// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';

const api = vi.hoisted(() => ({ apiGet: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../../chat.jsx', () => ({ ChatPanel: ({ onCreated, initialPrompt, launchKind }) => <button onClick={() => onCreated(`${launchKind}-1`)}>{launchKind}:{initialPrompt}</button> }));
vi.mock('../../fleet/teams.jsx', () => ({ FleetTeamsPanel: ({ onCreated, initialPrompt }) => <button onClick={() => onCreated('team-1')}>Team:{initialPrompt}</button> }));
vi.mock('../../dag/defsTab.jsx', () => ({ DefsTab: ({ onDispatched, initialPrompt }) => <button onClick={() => onDispatched('dag-1')}>DAG:{initialPrompt}</button> }));
vi.mock('../../todoPanel.jsx', () => ({ TodoPanel: ({ onCreated, initialPrompt }) => <button onClick={() => onCreated('todos-1')}>TODO 工作流:{initialPrompt}</button> }));
vi.mock('../../brain/workbench/launch.jsx', () => ({ Launch: ({ onCreated, initialPlan, initialPrompt }) => <button onClick={() => onCreated('brain-1')}>Brain:{initialPlan}:{initialPrompt}</button> }));
import { CapabilityLauncher } from './launcher.jsx';

it.each([
  ['agent', 'agent:任务\n\n说明', 'agent-1'],
  ['operator', 'operator:任务\n\n说明', 'operator-1'],
  ['team', 'Team:任务\n\n说明', 'team-1'],
  ['dag', 'DAG:任务\n\n说明', 'dag-1'],
  ['todos', 'TODO 工作流:任务\n\n说明', 'todos-1'],
])('routes %s creation through the native capability UI', (kind, label, executionId) => {
  const onCreated = vi.fn();
  render(<CapabilityLauncher kind={kind} onKind={vi.fn()} onCreated={onCreated} onNotice={vi.fn()} prompt={'任务\n\n说明'} />);
  fireEvent.click(screen.getByRole('button', { name: new RegExp(label.replace(/\s+/g, '\\s+')) }));
  expect(onCreated).toHaveBeenCalledWith(executionId);
});

it('uses a versioned brain plan and forwards its execution ID', async () => {
  api.apiGet.mockImplementation((path) => Promise.resolve(path === '/api/brain/library'
    ? { capabilities: [] }
    : { plans: [{ id: 'plan-a', title: '计划 A', latest_version: 2, schema_version: 7 }] }));
  const onCreated = vi.fn();
  render(<CapabilityLauncher kind="brain" onKind={vi.fn()} onCreated={onCreated} onNotice={vi.fn()} prompt="任务" />);
  await screen.findByRole('combobox', { name: '大脑计划' });
  fireEvent.mouseDown(screen.getByRole('combobox', { name: '大脑计划' }));
  fireEvent.click(await screen.findByText('计划 A', { selector: '.ant-select-item-option-content' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Brain:plan-a@2:任务' })).toBeTruthy());
  fireEvent.click(screen.getByRole('button', { name: 'Brain:plan-a@2:任务' }));
  expect(onCreated).toHaveBeenCalledWith('brain-1');
});
