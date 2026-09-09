import { useCallback, useRef } from 'react';

// Notification callback identity must never restart a resource load or erase a draft.
export function useEvent(callback) {
  const latest = useRef(callback);
  latest.current = callback;
  return useCallback((...args) => latest.current?.(...args), []);
}
