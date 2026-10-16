import { useEffect, useRef } from 'react';

const key = 'opencoderLocationIndex';
// Browsers change window.location before dispatching popstate. Publish only
// accepted locations so an unrelated render cannot unmount the draft before
// the guard receives that event, or while it restores the original entry.
let acceptedLocation = null;
export const locationSnapshot = () => acceptedLocation || window.location.href;
function entry(fallback = 0) {
  const state = window.history.state || {};
  const index = Number.isInteger(state[key]) ? state[key] : fallback;
  return { href: window.location.href, index, state: { ...state, [key]: index } };
}

export function pushLocation(href) {
  if (new URL(href, window.location.href).href === window.location.href) return;
  const current = entry();
  window.history.replaceState(current.state, '', current.href);
  window.history.pushState({ [key]: current.index + 1 }, '', href);
  window.dispatchEvent(new PopStateEvent('popstate'));
}

export function replaceLocation(href) {
  window.history.replaceState(entry().state, '', href);
  window.dispatchEvent(new PopStateEvent('popstate'));
}

// Restore the current history entry before asking, so Cancel keeps both the
// draft and the address. Confirm then replays the original Back/Forward step.
export function useHistoryGuard(guard) {
  const latest = useRef(guard);
  latest.current = guard;
  useEffect(() => {
    let current = entry();
    acceptedLocation = current.href;
    window.history.replaceState(current.state, '', current.href);
    let pending = null;
    let asking = false;
    let restoring = false;
    let approved = null;
    const ask = () => {
      if (asking || !pending) return;
      asking = true;
      const target = pending;
      latest.current.run(() => {
        pending = null; asking = false; approved = target.href;
        if (target.delta) window.history.go(-target.delta);
        else pushLocation(target.href);
      }, () => { pending = null; asking = false; });
    };
    const changed = (event) => {
      const next = entry(current.index + 1);
      if (next.href === current.href) {
        if (pending) { event.stopImmediatePropagation(); restoring = false; ask(); }
        else current = next;
        return;
      }
      // App writes are already confirmed by their close/menu handlers.
      if (!pending && (!event.isTrusted || approved === next.href || !latest.current.blocked())) {
        approved = null; current = next; acceptedLocation = next.href;
        window.history.replaceState(current.state, '', current.href);
        return;
      }
      event.stopImmediatePropagation();
      const known = Number.isInteger(window.history.state?.[key]);
      const delta = known ? current.index - next.index : 0;
      if (!pending) pending = { ...next, delta };
      if (delta) { if (!restoring) { restoring = true; window.history.go(delta); } }
      else { window.history.replaceState(current.state, '', current.href); ask(); }
    };
    window.addEventListener('popstate', changed, true);
    window.addEventListener('hashchange', changed, true);
    return () => {
      acceptedLocation = null;
      window.removeEventListener('popstate', changed, true);
      window.removeEventListener('hashchange', changed, true);
    };
  }, []);
}
