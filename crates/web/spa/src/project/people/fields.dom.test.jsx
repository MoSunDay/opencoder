// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
import { matchesColumn } from '../views/projectTable.jsx';

const api = vi.hoisted(() => ({ apiGet: vi.fn() }));
vi.mock('../../api.js', () => api);
import { MdEditDrawer } from '../views/mdDrawer.jsx';
import { PeopleSelect, peopleColumns } from './fields.jsx';

beforeEach(() => {
  api.apiGet.mockReset().mockResolvedValue({ people: [{ name: 'alice' }, { name: 'bob' }] });
});

it('edits independent owner and followers with the project form and preserves failed saves', async () => {
  const save = vi.fn().mockResolvedValue(false);
  render(<MdEditDrawer open initial={{ id: 'p', title: 'Release', owner_user_name: 'alice', follower_user_names: ['bob'] }} onOk={save} onCancel={vi.fn()} />);
  await waitFor(() => expect(screen.getByLabelText('负责人').disabled).toBe(false));
  fireEvent.mouseDown(screen.getByLabelText('负责人'));
  const options = await screen.findAllByText('bob');
  fireEvent.click(options.find((item) => item.classList.contains('ant-select-item-option-content')));
  fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: 'Changed' } });
  fireEvent.click(screen.getByRole('button', { name: /保\s*存/ }));
  await waitFor(() => expect(save).toHaveBeenCalledWith(expect.objectContaining({ title: 'Changed', owner_user_name: 'bob', follower_user_names: ['bob'] })));
  expect(screen.getByPlaceholderText('一句话标题').value).toBe('Changed');
});

it('starts without inherited people and supports explicit owner clearing', async () => {
  const save = vi.fn();
  const props = { open: true, title: '编辑', onOk: save, onCancel: vi.fn() };
  const view = render(<MdEditDrawer {...props} initial={{ id: 'p', title: 'Parent', owner_user_name: 'alice', follower_user_names: ['bob'] }} />);
  await waitFor(() => expect(screen.getByLabelText('负责人').disabled).toBe(false));
  fireEvent.click(screen.getByLabelText('负责人').closest('.ant-select').querySelector('.ant-select-clear'));
  fireEvent.click(screen.getByRole('button', { name: /保\s*存/ }));
  await waitFor(() => expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ owner_user_name: null, follower_user_names: ['bob'] })));
  view.rerender(<MdEditDrawer {...props} initial={null} />);
  fireEvent.change(screen.getByPlaceholderText('一句话标题'), { target: { value: 'Child' } });
  fireEvent.click(screen.getByRole('button', { name: /保\s*存/ }));
  await waitFor(() => expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ owner_user_name: null, follower_user_names: [] })));
});

it('shows directory errors, retains the selected person and retries', async () => {
  api.apiGet.mockRejectedValueOnce(new Error('directory unavailable'));
  render(<PeopleSelect label="负责人" value="alice" onChange={vi.fn()} />);
  expect(await screen.findByText('directory unavailable')).toBeTruthy();
  expect(screen.getByLabelText('负责人').disabled).toBe(true);
  expect(screen.getByText('alice')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '重试人员列表' }));
  await waitFor(() => expect(screen.getByLabelText('负责人').disabled).toBe(false));
  expect(screen.queryByText('directory unavailable')).toBeNull();
});

it('filters owner and any follower without counting or changing records', () => {
  const [owner, followers] = peopleColumns();
  const record = { owner_user_name: 'alice', follower_user_names: ['alice', 'bob'] };
  expect(matchesColumn(owner, record, ['alice'])).toBe(true);
  expect(matchesColumn(owner, record, ['bob'])).toBe(false);
  expect(matchesColumn(followers, record, ['bob'])).toBe(true);
  expect(matchesColumn(followers, record, ['missing'])).toBe(false);
});
