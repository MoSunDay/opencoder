import { App } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../../api";
import type { Entity } from "../../../types";
import type { TextAttribute, TextFormat } from "./types";

type Input = { env: string; entity?: Entity; items: TextAttribute[]; reload: () => Promise<void>; onChanged: (entity: Entity) => Promise<void> };
export function useTextDrafts({ env, entity, items, reload, onChanged }: Input) {
  const { message } = App.useApp();
  const key = `${env}/${entity?.id ?? ""}`;
  const current = useRef(key);
  current.current = key;
  const generation = useRef(0);
  const cache = useRef<Record<string, number>>({});
  const requests = useRef<Record<string, number>>({});
  const pending = useRef<Record<string, number>>({});
  const baseline = useRef<Record<string, string>>({});
  const [values, setValues] = useState<Record<string, string>>({});
  const [bodies, setBodies] = useState<Record<string, string>>({});
  const [formats, setFormats] = useState<Record<string, TextFormat>>({});
  const [editing, setEditing] = useState<Record<string, boolean>>({});
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [saving, setSaving] = useState<Record<string, boolean>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  useEffect(() => {
    generation.current += 1;
    cache.current = {}; requests.current = {}; pending.current = {}; baseline.current = {};
    setValues({}); setBodies({}); setFormats({}); setEditing({}); setLoading({}); setSaving({}); setErrors({});
  }, [key]);
  useEffect(() => () => { generation.current += 1; }, []);
  const load = useCallback(async (item: TextAttribute, force = false) => {
    if (!entity) return;
    const id = item.definition.id;
    const metadataRevision = item.current?.revision ?? 0;
    const revision = Math.max(metadataRevision, cache.current[id] ?? 0);
    if (!force && (pending.current[id] === revision || (cache.current[id] !== undefined && cache.current[id] >= revision))) return;
    const requestKey = key;
    const requestGeneration = generation.current;
    const request = (requests.current[id] ?? 0) + 1;
    requests.current[id] = request;
    pending.current[id] = revision;
    const valid = () => current.current === requestKey && generation.current === requestGeneration && requests.current[id] === request;
    setLoading((state) => ({ ...state, [id]: true }));
    setErrors((state) => ({ ...state, [id]: "" }));
    try {
      const content = revision ? await api.textContent(env, entity.id, id, revision) : { content: "", format: "md" as const };
      const body = content.content;
      if (!valid()) return;
      const value = item.definition.storage_mode === "nfs_path"
        ? metadataRevision < revision ? baseline.current[id] ?? "" : item.current?.content_path ?? "" : body;
      cache.current[id] = revision;
      baseline.current[id] = value;
      setBodies((state) => ({ ...state, [id]: body }));
      setFormats((state) => ({ ...state, [id]: content.format }));
      setValues((state) => ({ ...state, [id]: value }));
    } catch (reason) {
      if (valid()) setErrors((state) => ({ ...state, [id]: reason instanceof Error ? reason.message : "文本读取失败" }));
    } finally { if (valid()) { delete pending.current[id]; setLoading((state) => ({ ...state, [id]: false })); } }
  }, [env, entity?.id, key]);
  const versionKey = items.map((item) => `${item.definition.id}/${item.current?.revision ?? 0}`).join(",");
  useEffect(() => { for (const item of items) if (!editing[item.definition.id] && !saving[item.definition.id]) void load(item); }, [versionKey, load, editing, saving]);
  const save = async (item: TextAttribute) => {
    if (!entity) return;
    const id = item.definition.id;
    if (saving[id]) return;
    const requestKey = key;
    const requestGeneration = generation.current;
    const valid = () => current.current === requestKey && generation.current === requestGeneration;
    const value = values[id] ?? "";
    const expectedRevision = cache.current[id] ?? item.current?.revision ?? 0;
    const nfs = item.definition.storage_mode === "nfs_path";
    let committed = false;
    setSaving((state) => ({ ...state, [id]: true }));
    setErrors((state) => ({ ...state, [id]: "" }));
    try {
      const saved = nfs ? await api.setNfsPath(env, entity.id, id, { path: value, expected_revision: expectedRevision })
        : await api.setText(env, entity.id, id, { format: formats[id] === "html" ? "html" : "md", content: value, expected_revision: expectedRevision });
      if (!valid()) return;
      committed = true;
      baseline.current[id] = value;
      cache.current[id] = saved.revision;
      setBodies((state) => ({ ...state, [id]: nfs ? "" : value }));
      setEditing((state) => ({ ...state, [id]: false }));
      message.success("内容已保存");
      if (nfs) await load({ ...item, current: { ...item.current, revision: saved.revision, content_path: value } }, true);
      if (valid()) await reload();
      if (valid()) await onChanged(entity);
    } catch (reason) {
      if (valid()) setErrors((state) => ({ ...state, [id]: `${committed ? "内容已保存，详情刷新失败：" : ""}${reason instanceof Error ? reason.message : "保存失败"}` }));
    } finally { if (valid()) setSaving((state) => ({ ...state, [id]: false })); }
  };
  const dirty = Object.keys(editing).some((id) => editing[id] && (values[id] ?? "") !== (baseline.current[id] ?? ""));
  return { values, bodies, formats, editing, loading, saving, errors, dirty, save, refresh: reload, retry: (item: TextAttribute) => void load(item, true),
    edit: (id: string) => setEditing((state) => ({ ...state, [id]: true })),
    cancel: (id: string) => { setValues((state) => ({ ...state, [id]: baseline.current[id] ?? "" })); setEditing((state) => ({ ...state, [id]: false })); setErrors((state) => ({ ...state, [id]: "" })); },
    change: (id: string, value: string) => setValues((state) => ({ ...state, [id]: value })),
  };
}
