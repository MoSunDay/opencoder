import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';

export function useCapabilities() {
  const [state, setState] = useState({ capabilities: [], loading: true, error: '' });
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const request = new AbortController();
    setState((old) => ({ ...old, loading: true, error: '' }));
    apiGet('/api/brain/library', { signal: request.signal }).then((body) => {
      if (!request.signal.aborted) setState({ capabilities: body.capabilities || [], loading: false, error: '' });
    }).catch((error) => {
      if (!request.signal.aborted) setState({ capabilities: [], loading: false, error: error.message });
    });
    return () => request.abort();
  }, [revision]);
  return { ...state, reload: () => setRevision((value) => value + 1) };
}

export function capabilityOptions(capabilities) {
  return capabilities.map((cap) => ({ value: cap.id,
    label: `${cap.summary || cap.target} · ${cap.target}`,
    disabled: !!cap.unavailable_reason || !cap.definition,
  }));
}
