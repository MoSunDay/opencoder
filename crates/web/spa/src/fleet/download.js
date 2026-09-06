import { getState } from '../store.js';

const ARTIFACT_PATH = /^\/api\/executions\/[A-Za-z0-9_-]+\/artifact\?/;

function artifactPath(id, step, file) {
  const path = `/api/executions/${encodeURIComponent(id)}/artifact?step=${encodeURIComponent(step)}&file=${encodeURIComponent(file)}`;
  if (!ARTIFACT_PATH.test(path)) throw new Error('无效的产物下载路径');
  return path;
}

function randomId() {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID().replaceAll('-', '');
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

async function activeWorker() {
  if (!('serviceWorker' in navigator)) throw new Error('浏览器不支持流式文件下载');
  const registration = await navigator.serviceWorker.register('/static/download-sw.js', { scope: '/' });
  await navigator.serviceWorker.ready;
  if (!navigator.serviceWorker.controller) {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('下载服务接管页面超时')), 10_000);
      navigator.serviceWorker.addEventListener('controllerchange', () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
    });
  }
  return navigator.serviceWorker.controller
    || registration.active || registration.waiting || registration.installing;
}

let workerReady;
export function prepareArtifactDownloads() {
  if (!workerReady) workerReady = activeWorker().catch((error) => {
    workerReady = null;
    throw error;
  });
  return workerReady;
}

function register(worker, record) {
  return new Promise((resolve, reject) => {
    const channel = new MessageChannel();
    const timer = setTimeout(() => reject(new Error('下载服务初始化超时')), 10_000);
    channel.port1.onmessage = ({ data }) => {
      clearTimeout(timer);
      if (data?.ok) resolve();
      else reject(new Error(data?.error || '下载服务拒绝请求'));
    };
    worker.postMessage({ type: 'opencoder-download', ...record }, [channel.port2]);
  });
}

export async function downloadArtifact(id, step, file) {
  const token = getState().token;
  if (!token) throw new Error('请先登录');
  let worker = navigator.serviceWorker?.controller;
  const retainsUserGesture = !!worker;
  if (!worker) worker = await prepareArtifactDownloads();
  if (!worker) throw new Error('下载服务尚未就绪');
  const requestId = randomId();
  const popup = retainsUserGesture ? window.open('about:blank', '_blank') : null;
  if (retainsUserGesture && !popup) throw new Error('浏览器阻止了下载窗口');
  const registered = register(worker, {
    id: requestId, path: artifactPath(id, step, file), token,
  });
  if (retainsUserGesture) {
    await registered;
    popup.location.replace(`/__opencoder_download/${requestId}`);
  } else {
    await registered;
    const frame = document.createElement('iframe');
    frame.src = `/__opencoder_download/${requestId}`;
    frame.title = 'artifact-download';
    frame.hidden = true;
    document.body.appendChild(frame);
    setTimeout(() => frame.remove(), 60_000);
  }
  await registered;
}

export { artifactPath };
