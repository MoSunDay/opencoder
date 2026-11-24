import { useMemo, useSyncExternalStore } from 'react';
import { executionLink, linkedPage } from './links.js';
import { projectLink } from '../../project/navigation/links.js';
import { locationSnapshot } from './history.js';

function subscribe(changed) {
  window.addEventListener('hashchange', changed);
  window.addEventListener('popstate', changed);
  return () => {
    window.removeEventListener('hashchange', changed);
    window.removeEventListener('popstate', changed);
  };
}
export function useLinkedLocation() {
  const href = useSyncExternalStore(subscribe, locationSnapshot);
  return useMemo(() => {
    const url = new URL(href);
    return { page: linkedPage(url.search, url.hash), execution: executionLink(url.search, url.hash), project: projectLink(url.search, url.hash) };
  }, [href]);
}
