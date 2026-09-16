import {useCallback, useEffect, useRef, useState} from 'react';
import {apiGet, apiPost, apiPut} from '../api.js';
import {useEvent} from '../ui/editing/useEvent.js';
import {CATEGORIES, fileChanges, isDirty, resourceUrl, snapshotFiles} from './resourceModel.js';

export function useResources(name, onChanged, onDirtyChange) {
  const [entries,setEntries] = useState({});
  const alive = useRef(true); const generation = useRef(0);
  const changed = useEvent(onChanged); const reportDirty = useEvent(onDirtyChange);
  const apply = useCallback((cat, entry) => { if (alive.current) setEntries(previous => ({...previous,[cat]:entry})); },[]);
  const load = useCallback(async () => {
    const current = ++generation.current;
    setEntries({});
    await Promise.all(CATEGORIES.map(async ({cat}) => {
      try {
        const view = await apiGet(resourceUrl(name,cat)); const files = snapshotFiles(view);
        if (current === generation.current) apply(cat,{view,original:files,draft:files});
      } catch (error) { if (current === generation.current) apply(cat,{loadError:error.message}); }
    }));
  },[name,apply]);
  useEffect(() => { alive.current = true; load(); return () => { alive.current = false; generation.current++; }; },[load]);
  const dirty = Object.values(entries).some(isDirty);
  const busy = Object.values(entries).some(entry => entry.saving);
  useEffect(() => { reportDirty?.(dirty || busy); },[dirty,busy,reportDirty]);
  useEffect(() => {
    const beforeUnload = event => { if (dirty || busy) { event.preventDefault(); event.returnValue = ''; } };
    window.addEventListener('beforeunload',beforeUnload);
    return () => window.removeEventListener('beforeunload',beforeUnload);
  },[dirty,busy]);
  const edit = (cat, draft) => setEntries(previous => ({...previous,[cat]:{...previous[cat],draft,saved:false}}));
  const save = async (cat, version) => {
    const entry = entries[cat];
    if (!entry?.view || entry.loadError || entry.view.read_only || entry.saving) return;
    if (version !== undefined && isDirty(entry) && !window.confirm('恢复历史版本会丢弃当前页签的未保存内容，继续？')) return;
    apply(cat,{...entry,saving:true,error:''});
    try {
      const url = resourceUrl(name,cat);
      const view = version === undefined
        ? await apiPut(url,{baseline:entry.view.baseline,...fileChanges(entry.original,entry.draft)})
        : await apiPost(`${url}/restore`,{baseline:entry.view.baseline,version});
      const files = snapshotFiles(view); apply(cat,{view,original:files,draft:files,saved:true});
      changed?.();
    } catch (error) { apply(cat,{...entry,error:error.message,saving:false}); }
  };
  const refresh = () => { if (!busy && (!dirty || window.confirm('刷新会丢弃未保存内容，继续？'))) load(); };
  return {entries,edit,save,refresh,busy};
}
