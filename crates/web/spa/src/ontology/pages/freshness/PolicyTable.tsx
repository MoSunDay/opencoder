import { PlusOutlined } from "@ant-design/icons";
import { Alert, App, Button, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { api } from "../../api";
import { entityTypeOptions, searchableOptions } from "../../forms/selectOptions";
import type { EntityType, FreshnessPolicy } from "../../types";
import { DataTable, ModalForm, NumberField, SelectField, TextAreaField, TextField } from "../../ui";
import DangerMenu from "../admin/DangerMenu";
import ListFilters, { matchesRecord, type StatusFilter } from "../admin/ListFilters";
import { useResource } from "../admin/useResource";

type ExecutorValues = { kind: "agent" | "dag"; target: string; prompt_template: string };
type PolicyValues = { name: string; description?: string; entity_type_id: string; executor: ExecutorValues; concurrency: number; writeback_attribute_key?: string };
type Props = { env: string; canManage: boolean; onTriggered: (runId: string) => void };

export const PROMPT_PLACEHOLDER = "刷新提示词模板，可用占位符：{{entity.id}}、{{entity.name}}、{{entity.description}}、{{entity.data_level}}";

const policyBody = (values: PolicyValues) => ({
  ...values,
  description: values.description ?? "",
  writeback_attribute_key: values.writeback_attribute_key?.trim() || null,
});

function PolicyForm({ env, types, item, onSaved }: { env: string; types: EntityType[]; item?: FreshnessPolicy; onSaved: () => Promise<void> }) {
  const { message } = App.useApp();
  return <ModalForm<PolicyValues>
    title={item ? "编辑保鲜策略" : "新增保鲜策略"}
    trigger={item ? <Button type="link">编辑</Button> : <Button type="primary" icon={<PlusOutlined />}>新增</Button>}
    modalProps={{ destroyOnHidden: true }}
    initialValues={item ? {
      name: item.name, description: item.description, entity_type_id: item.entity_type_id,
      executor: { kind: item.executor.kind, target: item.executor.target, prompt_template: item.executor.prompt_template },
      concurrency: item.concurrency, writeback_attribute_key: item.writeback_attribute_key ?? undefined,
    } : { executor: { kind: "agent", target: "", prompt_template: "" }, concurrency: 1 }}
    onFinish={async (values) => {
      if (item) await api.updateFreshnessPolicy(env, item.id, { ...policyBody(values), is_deleted: false, expected_revision: item.revision });
      else await api.createFreshnessPolicy(env, policyBody(values));
      message.success(item ? "策略已更新" : "策略已创建");
      await onSaved();
    }}>
    <TextField name="name" label="名称" rules={[{ required: true }]} />
    <TextAreaField name="description" label="描述" />
    <SelectField name="entity_type_id" label="实体类型" rules={[{ required: true }]} showSearch
      fieldProps={searchableOptions} options={entityTypeOptions(types)} />
    <SelectField name={["executor", "kind"]} label="执行器类型" rules={[{ required: true }]}
      options={[{ value: "agent", label: "Agent" }, { value: "dag", label: "DAG" }]} />
    <TextField name={["executor", "target"]} label="执行目标" rules={[{ required: true }]} placeholder="Agent 名称或 DAG 标识" />
    <TextAreaField name={["executor", "prompt_template"]} label="提示词模板" rules={[{ required: true }]} placeholder={PROMPT_PLACEHOLDER} />
    <NumberField name="concurrency" label="并发数" rules={[{ required: true }]} fieldProps={{ min: 1, max: 30 }} />
    <TextField name="writeback_attribute_key" label="写回属性" placeholder="可选，评审通过后写回的文本属性 Key" />
  </ModalForm>;
}

export default function PolicyTable({ env, canManage, onTriggered }: Props) {
  const { message } = App.useApp();
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<StatusFilter>("active");
  const [page, setPage] = useState(1);
  const [triggering, setTriggering] = useState<string>();
  const resource = useResource(env, async () => {
    const [policies, types] = await Promise.all([api.freshnessPolicies(env, true), api.entityTypes(env, true)]);
    return { items: policies.items, types: types.items };
  });
  const { items = [], types = [] } = resource.data ?? {};
  const typeNames = Object.fromEntries(types.map((type) => [type.id, type.name]));
  useEffect(() => { setPage(1); }, [query, status]);
  const visible = items.filter((item) => matchesRecord(item, status, query, [item.name, item.description, typeNames[item.entity_type_id] ?? ""]));
  const trigger = async (item: FreshnessPolicy) => {
    setTriggering(item.id);
    try {
      const result = await api.triggerFreshnessRun(env, item.id);
      message.success(`运行已创建：${result.item.id}`);
      onTriggered(result.item.id);
    } catch (reason) { message.error(reason instanceof Error ? reason.message : "触发失败"); }
    finally { setTriggering(undefined); }
  };
  return <div>
    {resource.error ? <Alert type="error" showIcon message={resource.data ? "更新失败，当前显示上次结果" : "策略加载失败"} description={resource.error}
      action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <DataTable<FreshnessPolicy> scroll={{ x: 900 }} headerTitle="全部保鲜策略" rowKey="id" search={false}
      loading={resource.loading} dataSource={visible}
      pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }}
      options={{ reload: () => resource.reload() }}
      tableExtraRender={() => <ListFilters query={query} status={status} count={visible.length} onQuery={setQuery} onStatus={setStatus} />}
      toolBarRender={() => canManage ? [<PolicyForm key="new" env={env} types={types} onSaved={resource.reload} />] : []}
      columns={[
        { title: "名称", dataIndex: "name", width: 200, ellipsis: true, render: (_, row) => <Typography.Text strong>{row.name}</Typography.Text> },
        { title: "类型", width: 200, render: (_, row) => <Space size={4} wrap>
          <Tag color={row.executor.kind === "agent" ? "blue" : "purple"}>{row.executor.kind === "agent" ? "Agent" : "DAG"}</Tag>
          <Typography.Text type="secondary" ellipsis>{row.executor.target}</Typography.Text>
        </Space> },
        { title: "实体类型", width: 140, render: (_, row) => <Tag>{typeNames[row.entity_type_id] ?? row.entity_type_id}</Tag> },
        { title: "并发", dataIndex: "concurrency", width: 70 },
        { title: "写回属性", width: 140, render: (_, row) => row.writeback_attribute_key ? <Typography.Text code>{row.writeback_attribute_key}</Typography.Text> : "-" },
        { title: "状态", width: 80, render: (_, row) => row.is_deleted ? <Tag>已删除</Tag> : <Tag color="green">有效</Tag> },
        { title: "操作", width: 200, render: (_, row) => <Space size={0}>
          {canManage && !row.is_deleted ? <Button type="link" loading={triggering === row.id} onClick={() => void trigger(row)}>触发运行</Button> : null}
          {canManage && !row.is_deleted ? <PolicyForm env={env} types={types} item={row} onSaved={resource.reload} /> : null}
          {canManage && !row.is_deleted ? <DangerMenu label="删除策略" description="删除后不可再触发新的运行，历史运行保留。" onConfirm={async () => {
            await api.updateFreshnessPolicy(env, row.id, {
              name: row.name, description: row.description, entity_type_id: row.entity_type_id, executor: row.executor,
              concurrency: row.concurrency, writeback_attribute_key: row.writeback_attribute_key ?? null,
              is_deleted: true, expected_revision: row.revision,
            });
            message.success("策略已删除"); await resource.reload();
          }} /> : null}
        </Space> },
      ]} />
  </div>;
}
