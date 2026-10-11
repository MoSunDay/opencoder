export const ROLE_OPTIONS = [
  { value: 'editor', label: 'editor · 可编辑' },
  { value: 'viewer', label: 'viewer · 只读' },
];
export const ROLE_LABELS = { admin: 'admin · 启动管理员', editor: 'editor · 可编辑', viewer: 'viewer · 只读' };
export function matchesSearch(query, ...values) {
  const term = query.trim().toLocaleLowerCase();
  return !term || values.some((value) => String(value ?? '').toLocaleLowerCase().includes(term));
}
export function readUsers(response) {
  if (!Array.isArray(response?.users)) throw new Error('用户列表格式错误');
  return response.users;
}
