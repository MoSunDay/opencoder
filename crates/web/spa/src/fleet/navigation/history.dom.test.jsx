// @vitest-environment jsdom
import { App, ConfigProvider } from 'antd';
import { useState } from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
import { DraftGuardProvider, useDraftGuard } from '../../ontology/navigation/DraftGuard';
import { pushLocation, replaceLocation } from './history.js';
import { useLinkedLocation } from './location.js';

function Editor({ saving }) {
  const [value, setValue] = useState('');
  useDraftGuard(!!value, saving);
  return <input aria-label="草稿" value={value} onChange={(event) => setValue(event.target.value)} />;
}
function View({ saving }) {
  const [tick, setTick] = useState(0);
  const { project } = useLinkedLocation();
  return <>
    <button onClick={() => setTick(tick + 1)}>刷新界面 {tick}</button>
    <output>{project?.todoId || '项目'}</output>
    {project?.todoId && <Editor saving={saving} />}
  </>;
}
const mount = (saving = false) => render(<ConfigProvider theme={{ token: { motion: false } }}><App>
  <DraftGuardProvider><View saving={saving} /></DraftGuardProvider>
</App></ConfigProvider>);
beforeEach(() => window.history.replaceState(null, '', '/#project=p1'));
afterEach(() => { vi.restoreAllMocks(); window.history.replaceState(null, '', '/'); });
const confirm = async () => (await screen.findByText('有内容尚未保存', { selector: '.ant-modal-confirm-title' })).closest('[role="dialog"]');

it('does not expose a transient browser address before its history event is checked', () => {
  mount(); act(() => pushLocation('/#project=p1&todo=t1'));
  fireEvent.change(screen.getByLabelText('草稿'), { target: { value: '保留草稿' } });
  const state = window.history.state;
  // A browser traversal updates the address before it delivers popstate.
  window.history.replaceState(null, '', '/#project=p1');
  fireEvent.click(screen.getByRole('button', { name: '刷新界面 0' }));
  expect(screen.getByLabelText('草稿').value).toBe('保留草稿');
  window.history.replaceState(state, '', '/#project=p1&todo=t1');
});

it('preserves the mounted draft when React renders before the browser restores a blocked history entry', async () => {
  mount(); act(() => pushLocation('/#project=p1&todo=t1'));
  fireEvent.change(screen.getByLabelText('草稿'), { target: { value: '保留草稿' } });
  const go = window.history.go.bind(window.history);
  const paused = vi.spyOn(window.history, 'go').mockImplementation(() => {});
  act(() => window.history.back());
  await waitFor(() => expect(paused).toHaveBeenCalledWith(1));
  expect(window.location.hash).toBe('#project=p1');
  fireEvent.click(screen.getByRole('button', { name: '刷新界面 0' }));
  expect(screen.getByLabelText('草稿').value).toBe('保留草稿');
  expect(screen.getByRole('status').textContent).toBe('t1');
  paused.mockRestore(); act(() => go(1));
  fireEvent.click(within(await confirm()).getByRole('button', { name: '继续编辑' }));
  expect(window.location.hash).toBe('#project=p1&todo=t1');
  expect(screen.getByLabelText('草稿').value).toBe('保留草稿');
});

it('keeps a saving form on Back and allows Back once saving ends', async () => {
  const view = mount(true); act(() => pushLocation('/#project=p1&todo=t1'));
  act(() => window.history.back());
  await screen.findByText('内容正在保存，请稍候');
  expect(window.location.hash).toBe('#project=p1&todo=t1');
  expect(screen.getByLabelText('草稿')).toBeTruthy();
  view.rerender(<ConfigProvider theme={{ token: { motion: false } }}><App>
    <DraftGuardProvider><View saving={false} /></DraftGuardProvider>
  </App></ConfigProvider>);
  act(() => window.history.back());
  await waitFor(() => expect(screen.queryByLabelText('草稿')).toBeNull());
  expect(window.location.hash).toBe('#project=p1');
});

it('restores a multi-step Back after other pages replace their own address', async () => {
  mount();
  act(() => {
    pushLocation('/#execution=e1');
    replaceLocation('/?brain_run=b1');
    replaceLocation('/');
    pushLocation('/#todo=t1');
  });
  fireEvent.change(screen.getByLabelText('草稿'), { target: { value: '保留草稿' } });
  act(() => window.history.go(-2));
  fireEvent.click(within(await confirm()).getByRole('button', { name: '继续编辑' }));
  expect(window.location.hash).toBe('#todo=t1');
  expect(screen.getByLabelText('草稿').value).toBe('保留草稿');
});
