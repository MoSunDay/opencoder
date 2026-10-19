import { getState, urlFor } from '../store.js';

const ARTIFACT_PATH = /^\/api\/executions\/[A-Za-z0-9_-]+\/artifact\?/;

function artifactPath(id, step, file, index) {
  if (index != null && (!Number.isSafeInteger(index) || index < 0)) throw new Error('无效的实例索引');
  const path = `/api/executions/${encodeURIComponent(id)}/artifact?step=${encodeURIComponent(step)}&file=${encodeURIComponent(file)}`;
  if (!ARTIFACT_PATH.test(path)) throw new Error('无效的产物下载路径');
  return index == null ? path : `${path}&index=${index}`;
}

function randomId() {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID().replaceAll('-', '');
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

function downloadBase() {
  const url = new URL(urlFor('/'), window.location.href);
  if (url.origin !== window.location.origin) throw new Error('文件下载需要使用同源服务入口');
  return url.pathname.replace(/\/$/, '');
}

function downloadFrame(url) {
  const frame = document.createElement('iframe');
  frame.src = url;
  frame.title = 'artifact-download';
  frame.hidden = true;
  document.body.appendChild(frame);
  setTimeout(() => frame.remove(), 60_000);
}

function workerFor(base) {
  const worker = navigator.serviceWorker?.controller;
  return worker?.scriptURL === new URL(`${base}/static/download-sw.js`, window.location.href).href ? worker : null;
}

async function activeWorker(base) {
  if (!('serviceWorker' in navigator)) throw new Error('浏览器不支持流式文件下载');
  await navigator.serviceWorker.register(`${base}/static/download-sw.js`, { scope: `${base}/` });
  await navigator.serviceWorker.ready;
  if (!workerFor(base)) {
    await new Promise((resolve, reject) => {
      const changed = () => {
        if (!workerFor(base)) return;
        clearTimeout(timer);
        navigator.serviceWorker.removeEventListener('controllerchange', changed);
        resolve();
      };
      const timer = setTimeout(() => {
        navigator.serviceWorker.removeEventListener('controllerchange', changed);
        reject(new Error('下载服务接管页面超时'));
      }, 10_000);
      navigator.serviceWorker.addEventListener('controllerchange', changed);
    });
  }
  return workerFor(base);
}

let workerReady, workerBase;
export async function prepareArtifactDownloads() {
  const base = downloadBase();
  if (workerBase !== base) { workerReady = null; workerBase = base; }
  if (!workerReady) workerReady = activeWorker(base).catch((error) => {
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

export async function downloadArtifact(id, step, file, index) {
  const { token, authEnabled } = getState();
  if (!token && authEnabled !== false) throw new Error('请先登录');
  const base = downloadBase();
  const path = base + artifactPath(id, step, file, index);
  if (authEnabled === false) { downloadFrame(path); return; }
  let worker = workerFor(base);
  const retainsUserGesture = !!worker;
  if (!worker) worker = await prepareArtifactDownloads();
  if (!worker) throw new Error('下载服务尚未就绪');
  const requestId = randomId();
  const popup = retainsUserGesture ? window.open('about:blank', '_blank') : null;
  if (retainsUserGesture && !popup) throw new Error('浏览器阻止了下载窗口');
  const registered = register(worker, {
    id: requestId, path, token, clientUrl: window.location.href.split('#')[0],
  });
  if (retainsUserGesture) {
    await registered;
    popup.location.replace(`${base}/__opencoder_download/${requestId}`);
  } else {
    await registered;
    downloadFrame(`${base}/__opencoder_download/${requestId}`);
  }
  await registered;
}

export { artifactPath };
