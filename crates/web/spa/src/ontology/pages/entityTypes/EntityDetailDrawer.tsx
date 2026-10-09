import { Alert, Button, Collapse, Descriptions, Drawer, Empty, Grid, List, Skeleton, Space, Tabs, Tag, Tooltip, Typography } from "antd";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "../../api";
import type { AttributeDefinition, Entity, EntityType } from "../../types";
import EntityActions from "../EntityActions";
import StructuredAttributeList from "./StructuredAttributeList";
import TextAttributeSection from "./TextAttributeSection";
import type { StructuredAttributeRow, TextAttribute } from "./details/types";
import { useStructuredDrafts } from "./details/useStructuredDrafts";
import { useTextDrafts } from "./details/useTextDrafts";
import { useDraftGuard } from "../../navigation/DraftGuard";
import type { DetailView } from "../graph/details/useDetailNavigation";

type DetailTab = "basic" | "attributes" | "extension" | "source";
const TABS = [{ key: "basic", label: "基本信息" }, { key: "attributes", label: "普通属性" },
  { key: "extension", label: "拓展信息" }, { key: "source", label: "来源" }];
type Props = { env: string; entity?: Entity; entityType?: EntityType; canManage: boolean;
  onClose: () => void; onEntityChanged: (entity: Entity) => Promise<void>;
  relations?: ReactNode; onBack?: () => void; onObserve?: () => void; view?: DetailView; onView?: (view: DetailView) => void };

export default function EntityDetailDrawer({ env, entity, entityType, canManage, onClose, onEntityChanged, relations, onBack, onObserve, view, onView }: Props) {
  const screens = Grid.useBreakpoint();
  const [definitions, setDefinitions] = useState<AttributeDefinition[]>([]);
  const [rows, setRows] = useState<StructuredAttributeRow[]>([]);
  const [detailEntity, setDetailEntity] = useState<Entity>();
  const [textAttributes, setTextAttributes] = useState<TextAttribute[]>([]);
  const [actions, setActions] = useState<{ id: string; operation_type: string; operation: string }[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [activeTab, setActiveTab] = useState<DetailTab>("basic");
  const body = useRef<HTMLDivElement>(null);
  const restoredScroll = useRef<number>();
  const selection = useRef("");
  selection.current = `${env}/${entity?.id ?? ""}`;
  const generation = useRef(0);
  const reload = useCallback(async (item: Entity) => {
    const key = `${env}/${item.id}`;
    if (!entityType || selection.current !== key) return;
    const request = ++generation.current;
    const current = () => selection.current === key && generation.current === request;
    setError(""); setLoading(true);
    try {
      const [typeResult, detail] = await Promise.all([api.attributes(env, entityType.id), api.entity(env, item.id)]);
      if (!current()) return;
      setDefinitions(typeResult.items.filter((definition) => definition.kind !== "vector"));
      setDetailEntity(detail.item); setRows((detail.structured_attributes as StructuredAttributeRow[]) ?? []);
      setTextAttributes(detail.text_attributes ?? []); setActions(detail.actions ?? []);
    } catch (reason) { if (current()) setError(reason instanceof Error ? reason.message : "详情加载失败"); }
    finally { if (current()) setLoading(false); }
  }, [env, entityType?.id]);
  useEffect(() => {
    setActiveTab((view?.tab as DetailTab) || "basic"); restoredScroll.current = view?.scroll ?? 0;
    setDefinitions([]); setRows([]); setDetailEntity(undefined); setTextAttributes([]); setActions([]); setError("");
  }, [env, entity?.id]);
  useEffect(() => {
    if (entity && entityType) void reload(entity);
    return () => { generation.current += 1; };
  }, [entity?.id, entity?.revision, entityType?.id, reload]);
  const source = textAttributes.filter((item) => item.definition.attribute_role === "source");
  const extension = textAttributes.filter((item) => item.definition.attribute_role === "ext");
  const ordinaryText = textAttributes.filter((item) => !["source", "ext"].includes(item.definition.attribute_role ?? "custom"));
  const ordinary = definitions.filter((definition) => definition.kind !== "text" && !["source", "ext"].includes(definition.attribute_role ?? "custom"));
  const currentEntity = detailEntity?.id === entity?.id ? detailEntity : undefined;
  const items = activeTab === "attributes" ? ordinaryText : activeTab === "extension" ? extension : activeTab === "source" ? source : [];
  const drafts = useTextDrafts({ env, entity: currentEntity, items,
    reload: async () => { if (entity) await reload(entity); }, onChanged: onEntityChanged });
  const structured = useStructuredDrafts({ env, entity: currentEntity, definitions: ordinary, rows,
    onSaved: async () => { if (currentEntity) { await reload(currentEntity); await onEntityChanged(currentEntity); } } });
  const guard = useDraftGuard(structured.dirty || drafts.dirty, structured.saving || Object.values(drafts.saving).some(Boolean));
  useEffect(() => {
    if (loading || !currentEntity || restoredScroll.current === undefined || !body.current) return;
    if (items.some((item) => drafts.loading[item.definition.id] || (!Object.prototype.hasOwnProperty.call(drafts.values, item.definition.id) && !drafts.errors[item.definition.id]))) return;
    const frame = requestAnimationFrame(() => { if (body.current && restoredScroll.current !== undefined) { body.current.scrollTop = restoredScroll.current; restoredScroll.current = undefined; } });
    return () => cancelAnimationFrame(frame);
  }, [loading, currentEntity, activeTab, drafts.loading, drafts.values, drafts.errors]);
  const close = () => guard.run(onClose);
  const tabItems = TABS.map((tab) => {
    const definitions = tab.key === "attributes" ? ordinaryText : tab.key === "extension" ? extension : tab.key === "source" ? source : [];
    const changed = (tab.key === "attributes" && structured.dirty) || definitions.some((item) => drafts.editing[item.definition.id]);
    return { ...tab, label: <Space size={4}>{tab.label}{changed ? <Tag color="gold" style={{ margin: 0 }}>编辑中</Tag> : null}</Space> };
  });
  return <Drawer title={<Tooltip title={entity?.name}><Typography.Text strong ellipsis style={{ maxWidth: "100%" }}>{entity?.name}</Typography.Text></Tooltip>}
    width={screens.md ? 640 : "100%"} open={Boolean(entity)} onClose={close} destroyOnHidden
    styles={{ body: { display: "flex", flexDirection: "column", minHeight: 0, paddingTop: 12, overflow: "hidden" } }}
    extra={canManage && currentEntity && !currentEntity.is_deleted ? <EntityActions key={`${currentEntity.id}/${currentEntity.revision}`} env={env} entity={currentEntity}
      onDone={async (item) => { await reload(item); await onEntityChanged(item); }} /> : null}>
    {error ? <Alert type="error" showIcon message={error} action={<Button onClick={() => { if (entity) void reload(entity); }}>重试</Button>} /> : null}
    {onBack || onObserve ? <Space style={{ marginBottom: 8 }}>
      {onBack ? <Button onClick={() => guard.run(onBack)}>返回上一项</Button> : null}
      {onObserve ? <Button onClick={() => guard.run(onObserve)}>以此为中心观测</Button> : null}
    </Space> : null}
    <Tabs activeKey={activeTab} onChange={(key) => { setActiveTab(key as DetailTab); if (body.current) body.current.scrollTop = 0; onView?.({ tab: key, scroll: 0 }); }} items={tabItems} style={{ flexShrink: 0 }} />
    <div ref={body} className="entity-detail-body" onScroll={() => onView?.({ tab: activeTab, scroll: body.current?.scrollTop ?? 0 })}>
      {!currentEntity || !entityType ? loading ? <Skeleton active /> : error ? null : <Empty /> : <>
        {activeTab === "basic" ? <>
          <Descriptions column={1} size="small" items={[
            { key: "type", label: "类型", children: <Tag>{entityType.name}</Tag> },
            { key: "description", label: "描述", children: currentEntity.description || "尚未填写描述" },
            { key: "actions", label: "支持的 Action", children: actions.length ? <Space wrap>{actions.map((action) =>
              <Tag key={action.id}>{action.operation_type}: {action.operation}</Tag>)}</Space> : "无" },
          ]} />
          {relations}
          <Collapse size="small" style={{ marginTop: 16 }} items={[{ key: "metadata", label: "记录信息", children:
            <Descriptions column={1} size="small" items={[
              { key: "id", label: "ID", children: <Typography.Text copyable>{currentEntity.id}</Typography.Text> },
              { key: "revision", label: "版本", children: currentEntity.revision },
            ]} /> }]} />
        </> : null}
        {activeTab === "attributes" ? <Space direction="vertical" size="middle" style={{ width: "100%" }}>
          {ordinary.length ? <StructuredAttributeList definitions={ordinary} canManage={canManage} state={structured} /> : null}
          {!ordinary.length && !ordinaryText.length ? <Empty description="暂无普通属性" /> : null}
          <TextAttributeSection items={ordinaryText} canManage={canManage} drafts={drafts} />
        </Space> : null}
        {activeTab === "extension" ? extension.length ? <TextAttributeSection items={extension} canManage={canManage} drafts={drafts} /> : <Empty description="暂无拓展信息" /> : null}
        {activeTab === "source" ? <>
          <Typography.Paragraph type="secondary">{source.some((item) => (item.current?.bytes ?? 0) > 0) ? "已维护" : "待完善"}</Typography.Paragraph>
          {source.length ? <TextAttributeSection items={source} canManage={canManage} drafts={drafts} /> : <Empty description="暂无来源定义" />}
        </> : null}
      </>}
    </div>
  </Drawer>;
}
