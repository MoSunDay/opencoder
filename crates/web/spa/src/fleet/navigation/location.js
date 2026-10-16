import { useMemo, useSyncExternalStore } from 'react';
import { executionLink, linkedPage } from './links.js';

function subscribe(changed) {
  window.addEventListener('hashchange', changed);
  window.addEventListener('popstate', changed);
  return () => {
    window.removeEventListener('hashchange', changed);
    window.removeEventListener('popstate', changed);
  };
}
const snapshot = () => window.location.href;

export function useLinkedLocation() {
  const href = useSyncExternalStore(subscribe, snapshot);
  return useMemo(() => {
    const url = new URL(href);
    return { page: linkedPage(url.search, url.hash), execution: executionLink(url.search, url.hash) };
  }, [href]);
}
