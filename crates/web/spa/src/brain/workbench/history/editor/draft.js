import { useLayoutEffect, useRef, useState } from 'react';
import { checkDraftPlan, createDraft } from './model.js';

export function draftKey(owner, version) {
  return `oc:brain:plan-draft:${encodeURIComponent(owner)}:${version ? `${version.id}@${version.version}` : 'new'}`;
}
export const legacyDraftKey = (key) => `${key}:legacy-v1`;
export function readDraft(key, version, storage = localStorage) {
  const raw = storage.getItem(key);
  if (raw === null) return createDraft(version);
  let draft;
  try { draft = JSON.parse(raw); } catch { throw new Error('浏览器草稿缓存已损坏，可丢弃后重新开始'); }
  if (!draft?.version?.id || !Array.isArray(draft.version.plan?.instances) || !draft.positions || !draft.raw || !draft.metadata) throw new Error('浏览器草稿格式无效（可能来自旧版协议缓存），原始缓存已保留，可丢弃后重新开始');
  try { checkDraftPlan(draft.version.plan); } catch (e) { throw new Error(`${e.message}；缓存可能来自旧版协议，可丢弃后重新开始`); }
  return draft;
}
export function useDraft(key, version) {
  const [initial] = useState(() => {
    try { return { draft: readDraft(key, version), error: '' }; }
    catch (e) { return { draft: null, error: e.message }; }
  });
  const [draft, setDraft] = useState(initial.draft);
  const [error, setError] = useState(initial.error);
  const cleared = useRef(false);
  const persist = () => {
    if (!draft) return false;
    try { cleared.current = false; localStorage.setItem(key, JSON.stringify(draft)); setError(''); return true; }
    catch (e) { setError(`草稿未写入浏览器：${e.message}`); return false; }
  };
  useLayoutEffect(() => { if (!cleared.current) persist(); }, [key, draft]); // synchronous flush before navigation/reload
  const retry = () => { try { cleared.current = false; setDraft(readDraft(key, version)); setError(''); } catch (e) { setError(e.message); } };
  const clear = () => { cleared.current = true; localStorage.removeItem(key); };
  // 旧协议残留缓存的逃生口：原文备份到 legacy 槽位后丢弃，从空计划/版本快照重新开始。
  const discard = () => {
    const raw = localStorage.getItem(key);
    if (raw !== null) localStorage.setItem(legacyDraftKey(key), raw);
    localStorage.removeItem(key);
    cleared.current = false;
    setDraft(createDraft(version));
    setError('');
  };
  return { draft, setDraft, error, persist, retry, clear, discard };
}
