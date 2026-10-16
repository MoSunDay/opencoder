// @vitest-environment jsdom
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { expect, it } from 'vitest';
import { bootUrlCredential, getState, installFetchRouter, mountApp, setCredentials, setState } from './test/appShell.jsx';

it('opens a shared project ahead of a saved page, follows history and clears the link on menu navigation', async () => {
  installFetchRouter({ withGoal: true });
  window.history.replaceState(null, '', '/#project=g-seed');
  localStorage.setItem('oc_nav_page', JSON.stringify('nodes'));
  setCredentials('smoke-token', '');
  await mountApp();
  await screen.findByText('项目 · 既有目标', { selector: '.ant-drawer-title' });
  act(() => setState({ page: 'topics' }));
  expect(screen.getByText('项目 · 既有目标', { selector: '.ant-drawer-title' })).toBeTruthy();
  expect(getState().page).toBe('project');
  fireEvent.click(within(document.querySelector('.fleet-sidebar')).getByRole('tab', { name: '后台管理', exact: true }));
  await waitFor(() => expect(window.location.hash).toBe(''));
  expect(screen.queryByText('项目 · 既有目标', { selector: '.ant-drawer-title' })).toBeNull();
  act(() => { window.location.hash = 'project=g-seed'; });
  await screen.findByText('项目 · 既有目标', { selector: '.ant-drawer-title' });
  expect(getState().page).toBe('project');
});

it('retains the shared target and history when link login consumes its credential', async () => {
  installFetchRouter({ withGoal: true });
  window.history.replaceState({ opencoderLocationIndex: 7 }, '', '/?token=fixture-token#project=g-seed');
  bootUrlCredential();
  await mountApp();
  await screen.findByText('项目 · 既有目标', { selector: '.ant-drawer-title' });
  expect(window.location.search).toBe('');
  expect(window.location.hash).toBe('#project=g-seed');
  expect(window.history.state.opencoderLocationIndex).toBe(7);
});
