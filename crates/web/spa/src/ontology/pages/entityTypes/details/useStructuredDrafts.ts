import { App } from "antd";
import { useEffect, useRef, useState } from "react";
import { api } from "../../../api";
import type { AttributeDefinition, Entity } from "../../../types";
import type { StructuredAttributeRow, StructuredDraft } from "./types";

type Input = { env: string; entity?: Entity; definitions: AttributeDefinition[];
  rows: StructuredAttributeRow[]; onSaved: () => Promise<void> };

function comparable(kind: AttributeDefinition["kind"], value: unknown): unknown {
  if (kind === "json" && typeof value === "string") {
    try { return JSON.parse(value); } catch { return value; }
  }
  if (kind === "boolean") return Boolean(value);
  if (kind === "integer" || kind === "float") return value ?? null;
  return value ?? "";
}

function updateDraft(drafts: Record<string, StructuredDraft>, definition: AttributeDefinition,
  row: StructuredAttributeRow | undefined, value: unknown): Record<string, StructuredDraft> {
  const id = definition.id;
  const original = drafts[id] ? drafts[id].original : row?.is_deleted ? undefined : row?.value;
  const next = { ...drafts };
  if (JSON.stringify(comparable(definition.kind, value)) === JSON.stringify(comparable(definition.kind, original))) delete next[id];
  else next[id] = { value, original, revision: drafts[id]?.revision ?? row?.revision ?? 0 };
  return next;
}

export function useStructuredDrafts({ env, entity, definitions, rows, onSaved }: Input) {
  const { message } = App.useApp();
  const key = `${env}/${entity?.id ?? ""}`;
  const current = useRef(key);
  current.current = key;
  const generation = useRef(0);
  const writing = useRef(false);
  const [drafts, setDrafts] = useState<Record<string, StructuredDraft>>({});
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    generation.current += 1;
    writing.current = false;
    setDrafts({}); setSaving(false);
  }, [key]);
  useEffect(() => () => { generation.current += 1; }, []);
  const submit = async () => {
    if (!entity || writing.current || !Object.keys(drafts).length) return;
    const requestKey = key;
    const requestGeneration = generation.current;
    const valid = () => current.current === requestKey && generation.current === requestGeneration;
    writing.current = true; setSaving(true);
    try {
      for (const [id, draft] of Object.entries(drafts)) {
        if (!valid()) return;
        const definition = definitions.find((item) => item.id === id);
        if (!definition) throw new Error("属性定义已变化，请刷新详情");
        await api.setAttribute(env, entity.id, id, {
          kind: definition.kind,
          value: definition.kind === "json" && typeof draft.value === "string" ? JSON.parse(draft.value) : draft.value,
          is_deleted: false, expected_revision: draft.revision,
        });
        if (!valid()) return;
        setDrafts((previous) => { const next = { ...previous }; delete next[id]; return next; });
      }
      message.success("属性已保存");
    } catch (reason) {
      if (valid()) message.error(reason instanceof Error ? reason.message : "保存失败，请稍后重试");
    } finally {
      if (valid()) {
        try { await onSaved(); }
        catch (reason) { if (valid()) message.error(reason instanceof Error ? reason.message : "属性刷新失败"); }
        finally { if (valid()) { writing.current = false; setSaving(false); } }
      }
    }
  };
  return {
    drafts, saving, dirty: Object.keys(drafts).length > 0, submit,
    valueOf: (id: string) => drafts[id] ? drafts[id].value : rows.find((row) => row.attribute_definition_id === id && !row.is_deleted)?.value,
    change: (definition: AttributeDefinition, value: unknown) => setDrafts((previous) =>
      updateDraft(previous, definition, rows.find((row) => row.attribute_definition_id === definition.id), value)),
  };
}
