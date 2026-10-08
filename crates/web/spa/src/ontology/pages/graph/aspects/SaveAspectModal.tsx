import { App, Alert, Button, Form, Input, Modal, Popconfirm, Radio, Select, Space } from "antd";
import { useEffect, useMemo, useState } from "react";
import type { EntityType, GraphAspect, RelationshipType } from "../../../types";
import { relationshipTypeLabel } from "../observationSelection";
import { useDraftGuard } from "../../../navigation/DraftGuard";
import type { GraphAspectInput, GraphAspectPatch } from "./useGraphAspects";

type SaveMode = "create" | "update";
type AspectFormValues = { name: string; description?: string };

type Props = {
  expandNeighbors?: boolean;
  entityTypes: EntityType[];
  relationshipTypes: RelationshipType[];
  entityTypeIds: string[];
  relationshipTypeIds: string[];
  centerIds: string[];
  upstreamDepth: number;
  downstreamDepth: number;
  aspects: GraphAspect[];
  open: boolean;
  currentAspect?: GraphAspect;
  onClose: () => void;
  onCreate: (value: GraphAspectInput) => Promise<unknown>;
  onUpdate: (id: string, value: GraphAspectPatch) => Promise<unknown>;
  onDelete: (id: string, expectedRevision: number) => Promise<unknown>;
};

/** 把切面测试页当前的实体类型 + 关系类型选择保存为命名切面：新建 / 覆盖已有 / 删除已有。 */
export default function SaveAspectModal({
  entityTypes, relationshipTypes, entityTypeIds, relationshipTypeIds, centerIds, upstreamDepth, downstreamDepth,
  aspects, open, currentAspect, onClose, onCreate, onUpdate, onDelete, expandNeighbors = false,
}: Props) {
  const { message } = App.useApp();
  const [form] = Form.useForm<AspectFormValues>();
  const [mode, setMode] = useState<SaveMode>("create");
  const [targetId, setTargetId] = useState<string>();
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [dirty, setDirty] = useState(false);
  const guard = useDraftGuard(open && dirty, open && (saving || deleting));
  const close = () => guard.run(onClose);
  const target = useMemo(() => aspects.find((item) => item.id === targetId), [aspects, targetId]);
  const scopeIncomplete = !entityTypeIds.length || (!expandNeighbors && !relationshipTypeIds.length);
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);

  useEffect(() => {
    if (!open) return;
    setMode("create");
    setTargetId(undefined);
    form.resetFields();
    setDirty(false);
  }, [open, form]);

  const handleTargetChange = (id?: string) => {
    setTargetId(id);
    const selected = aspects.find((item) => item.id === id);
    form.setFieldsValue({ name: selected?.name ?? "", description: selected?.description ?? "" });
  };

  const handleModeChange = (next: SaveMode) => {
    setMode(next);
    setTargetId(next === "update" ? currentAspect?.id : undefined);
    form.setFieldsValue({ name: next === "update" ? currentAspect?.name ?? "" : "", description: next === "update" ? currentAspect?.description ?? "" : "" });
  };

  const handleFinish = async (values: AspectFormValues) => {
    if (scopeIncomplete || saving || deleting) return;
    if (mode === "update" && !target) { message.error("请选择要更新的切面"); return; }
    const payload = { name: values.name, description: values.description, entity_type_ids: entityTypeIds,
      relationship_type_ids: relationshipTypeIds, default_center_ids: centerIds,
      default_upstream_depth: upstreamDepth, default_downstream_depth: downstreamDepth };
    try {
      setSaving(true);
      if (mode === "create") await onCreate(payload);
      else if (target) await onUpdate(target.id, { ...payload, is_deleted: false, expected_revision: target.revision });
      message.success("切面已保存");
      setDirty(false);
      onClose();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : "切面保存失败");
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!target || saving || deleting) return;
    try {
      setDeleting(true);
      await onDelete(target.id, target.revision);
      message.success("切面已删除");
      onClose();
    } catch (reason) {
      message.error(reason instanceof Error ? reason.message : "切面删除失败");
    } finally {
      setDeleting(false);
    }
  };

  const scopeSummary = `将保存当前观测范围：实体类型 ${entityTypeIds.map((id) => typeNames[id] ?? id).join("、") || "未选择"}；关系类型 ${
    relationshipTypeIds.map((id) => {
      const item = relationshipTypes.find((type) => type.id === id);
      return item ? relationshipTypeLabel(item, typeNames) : id;
    }).join("、") || "全部关系"}；默认中心 ${centerIds.length ? `${centerIds.length} 个` : "整个切面"}；上游 ${upstreamDepth} 跳、下游 ${downstreamDepth} 跳`;

  return <Modal title="保存切面" open={open} onCancel={close} destroyOnHidden maskClosable={false} closable={!saving && !deleting} keyboard={!saving && !deleting}
    footer={<div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
      {mode === "update" && target
        ? <Popconfirm title="确认删除该切面？" description="删除后切面观测将不再可选" okText="删除" cancelText="取消"
            onConfirm={() => void handleDelete()}>
            <Button danger loading={deleting} disabled={saving}>删除该切面</Button>
          </Popconfirm>
        : <span />}
      <Space>
        <Button onClick={close} disabled={saving || deleting}>取消</Button>
        <Button type="primary" loading={saving} disabled={scopeIncomplete || deleting || (mode === "update" && !target)} onClick={() => void form.submit()}>保存</Button>
      </Space>
    </div>}>
    <Alert type="info" showIcon message={scopeSummary} style={{ marginBottom: 16 }} />
    <Form form={form} layout="vertical" onFinish={handleFinish} onValuesChange={() => setDirty(true)} disabled={saving || deleting}>
      <Form.Item label="保存方式">
        <Radio.Group value={mode} onChange={(event) => handleModeChange(event.target.value)}
          options={[{ value: "create", label: "新建切面" }, { value: "update", label: "更新已有切面" }]} />
      </Form.Item>
      {mode === "update" ? <Form.Item label="已有切面" required style={{ marginBottom: 16 }}>
        <Select aria-label="已有切面选择" showSearch optionFilterProp="label" placeholder="请选择要覆盖的切面"
          value={targetId} onChange={handleTargetChange} options={aspects.map((item) => ({ value: item.id, label: item.name }))} />
      </Form.Item> : null}
      <Form.Item label="名称" name="name" rules={[{ required: true, message: "请输入切面名称" }]}>
        <Input placeholder="请输入切面名称" />
      </Form.Item>
      <Form.Item label="描述" name="description">
        <Input.TextArea placeholder="请输入切面描述" rows={2} />
      </Form.Item>
    </Form>
  </Modal>;
}
