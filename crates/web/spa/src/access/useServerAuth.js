import { useEffect } from 'react';
import { apiGet } from '../api.js';
import { setState, useStore } from '../store.js';

export function useServerAuth() {
  const { base, token, authEnabled } = useStore();
  useEffect(() => {
    let live = true;
    setState({ authEnabled: null });
    apiGet('/api/auth/status')
      .then((status) => { if (live) setState({ authEnabled: status?.enabled !== false }); })
      .catch(() => { if (live) setState({ authEnabled: true }); });
    return () => { live = false; };
  }, [base, token]);
  return authEnabled;
}
