import { Select } from 'antd';
import { useEffect, useState } from 'react';
import { apiPatch } from '../../api.js';
import { err } from '../../notice.js';
import { searchSelect } from '../model/relations.js';

// Keep the chosen association visible until the refreshed snapshot acknowledges it.
export function RelationSelect({ path, field, value, options, refresh, onNotice, label, disabled }) {
  return <RelationEditor key={JSON.stringify([path, field])} {...{ path, field, value, options, refresh, onNotice, label, disabled }} />;
}

function RelationEditor({ path, field, value, options, refresh, onNotice, label, disabled }) {
  const [pending, setPending] = useState(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (pending && (value ?? null) === pending.value) setPending(null);
  }, [value, pending]);
  const change = async (next) => {
    if (busy) return;
    const selected = next ?? null;
    setBusy(true); setPending({ value: selected });
    try {
      await apiPatch(path, { [field]: selected });
      await refresh();
    } catch (e) {
      setPending(null); onNotice(err('关联保存失败: ' + e.message));
    } finally { setBusy(false); }
  };
  return <Select {...searchSelect} aria-label={label} placeholder="未关联"
    style={{ minWidth: 200, maxWidth: '100%' }} options={options}
    value={pending ? pending.value : value} disabled={busy || disabled} loading={busy} onChange={change} />;
}
