// setup-dom.js — jsdom shims for the DOM smoke tests. Import ONLY from files
// that declare `// @vitest-environment jsdom`; the frozen pure-node suites
// (reduce.test.js / sign.test.js) never import this module, so they keep
// running DOM-less and byte-identical.
//
// Everything here is a global stub, never an app behavior change: antd 6 and
// the rc-* components beneath it assume browser APIs that jsdom does not
// implement (media queries, observers, scrolling, animation, network).

import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

/// Assign `value` for `name` on `target` only when missing, so a future jsdom
/// that implements an API natively wins over the shim.
function shimIfMissing(target, name, value) {
  if (target[name] === undefined || target[name] === null) {
    Object.defineProperty(target, name, { configurable: true, writable: true, value });
  }
}

// No manual IS_REACT_ACT_ENVIRONMENT preset here: React Testing Library's
// act-compat sets/restores that flag around every act() and waitFor() scope.
// Presetting it globally would flag main.jsx's import-time root.render() as
// an out-of-act update and print spurious act(...) warnings.

// antd responsive observers (Sider breakpoint, Grid, Menu) probe matchMedia;
// the shim always reports "no match", which is fine for landmark assertions.
const matchMediaShim = (query) => ({
  matches: false,
  media: String(query || ''),
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});
shimIfMissing(window, 'matchMedia', matchMediaShim);
shimIfMissing(globalThis, 'matchMedia', matchMediaShim);

// rc-table / Tooltip / Modal measure and observe DOM nodes. The constructor is
// a plain function (repo rule: no class syntax) — `new Shim()` returns the
// observer object, which is all callers ever touch.
const resizeObserverShim = function resizeObserverShim() {
  return {
    observe: () => {},
    unobserve: () => {},
    disconnect: () => {},
  };
};
shimIfMissing(window, 'ResizeObserver', resizeObserverShim);
shimIfMissing(globalThis, 'ResizeObserver', resizeObserverShim);

// @ant-design/x Bubble.List probes IntersectionObserver in its scroll hook
// (bubble/hooks/useCompatibleScroll.js) to decide whether auto-scroll may
// scroll the viewport. Same shape as the ResizeObserver shim above.
const intersectionObserverShim = function intersectionObserverShim() {
  return {
    observe: () => {},
    unobserve: () => {},
    disconnect: () => {},
    takeRecords: () => [],
  };
};
shimIfMissing(window, 'IntersectionObserver', intersectionObserverShim);
shimIfMissing(globalThis, 'IntersectionObserver', intersectionObserverShim);

// jsdom has no layout engine: scrolling and Web Animations do not exist.
const noop = () => {};
const animationShim = () => ({ cancel: noop, finished: Promise.resolve() });
shimIfMissing(Element.prototype, 'scrollIntoView', noop);
shimIfMissing(Element.prototype, 'scrollTo', noop);
shimIfMissing(Element.prototype, 'animate', animationShim);

// Tests must never touch a real network. main.jsx auto-mounts <App/> at import
// time (see app.dom.test.js) and its effects fetch immediately, so fetch is
// replaced unconditionally with a harmless resolver; the jsdom test file then
// overrides it with a URL-routed mock via vi.stubGlobal.
const harmlessFetch = () => Promise.resolve({
  ok: true,
  status: 200,
  json: () => Promise.resolve({}),
  text: () => Promise.resolve(''),
});
window.fetch = harmlessFetch;
globalThis.fetch = harmlessFetch;

// main.jsx ends with createRoot(document.getElementById('root')).render(<App/>)
// executed at import time — without a #root node that import would throw
// ("Target container is not a DOM element"). app.dom.test.js unmounts that
// stray root right after the import.
if (!document.getElementById('root')) {
  const rootFixture = document.createElement('div');
  rootFixture.id = 'root';
  document.body.appendChild(rootFixture);
}

// React Testing Library does not register auto-cleanup because vitest globals
// are off in this repo — unmount and reset the DOM between tests explicitly.
afterEach(() => {
  cleanup();
});
