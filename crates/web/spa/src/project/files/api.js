import { authFetch } from '../../api.js';

export function fileUrl(todoId, path) {
  return `/api/project/todos/${encodeURIComponent(todoId)}/file?${new URLSearchParams({ path })}`;
}
async function check(response) {
  if (response.ok) return response;
  const body = await response.json().catch(() => null);
  throw new Error(body?.error || `HTTP ${response.status}`);
}
export async function uploadFile(todoId, path, file, signal) {
  const response = await authFetch('PUT', fileUrl(todoId, path), undefined,
    { rawBody: file, contentType: 'application/octet-stream', signal });
  return (await check(response)).json();
}
export async function downloadFile(todoId, path, signal) {
  const response = await check(await authFetch('GET', fileUrl(todoId, path), undefined, { signal }));
  const url = URL.createObjectURL(await response.blob());
  try {
    const anchor = document.createElement('a');
    anchor.href = url; anchor.download = path.split('/').at(-1);
    document.body.append(anchor); anchor.click(); anchor.remove();
  } finally { setTimeout(() => URL.revokeObjectURL(url), 1000); }
}
