import { Button, Input, InputNumber, List, Space, Switch, Tag, Typography } from "antd";
import type { AttributeDefinition } from "../../types";
import type { useStructuredDrafts } from "./details/useStructuredDrafts";

type Props = {
  definitions: AttributeDefinition[];
  canManage: boolean;
  state: ReturnType<typeof useStructuredDrafts>;
};

export function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

export default function StructuredAttributeList({
  definitions,
  canManage,
  state,
}: Props) {
  const { drafts, saving, valueOf, change, submit } = state;

  const renderEditor = (definition: AttributeDefinition) => {
    const id = definition.id;
    const value = valueOf(id);
    if (!canManage) {
      return <Typography.Text>{formatValue(value)}</Typography.Text>;
    }
    switch (definition.kind) {
      case "boolean":
        return (
          <Switch
            disabled={saving}
            checked={Boolean(value)}
            onChange={(checked) => change(definition, checked)}
            aria-label={`编辑 ${definition.name}`}
          />
        );
      case "integer":
      case "float":
        return (
          <InputNumber
            disabled={saving}
            value={typeof value === "number" ? value : undefined}
            precision={definition.kind === "integer" ? 0 : undefined}
            style={{ width: "100%" }}
            onChange={(next) => change(definition, next ?? null)}
            aria-label={`编辑 ${definition.name}`}
          />
        );
      default:
        return (
          <Input
            disabled={saving}
            value={value === undefined || value === null ? "" : typeof value === "object" ? JSON.stringify(value) : String(value)}
            onChange={(event) => change(definition, event.target.value)}
            aria-label={`编辑 ${definition.name}`}
          />
        );
    }
  };

  return (
    <List
      header={
        <Space style={{ width: "100%", justifyContent: "space-between" }}>
          <Typography.Text strong>普通属性</Typography.Text>
          <Button
            type="primary"
            loading={saving}
            disabled={!state.dirty || !canManage}
            autoInsertSpace={false}
            onClick={() => void submit()}
          >
            提交
          </Button>
        </Space>
      }
      dataSource={definitions}
      renderItem={(definition) => {
        const id = definition.id;
        const pending = drafts[id] !== undefined;
        return (
          <List.Item key={id}>
            <div style={{ width: "100%" }}>
              <Space direction="vertical" style={{ width: "100%" }} size={2}>
                <Space>
                  <Typography.Text strong>{definition.name}</Typography.Text>
                  <Tag>{definition.kind}</Tag>
                  {definition.required ? <Tag color="red">必填</Tag> : null}
                  {pending ? <Tag color="orange">未保存</Tag> : null}
                </Space>
                <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                  {definition.description || `键：${definition.attribute_key}`}
                </Typography.Text>
                {renderEditor(definition)}
              </Space>
            </div>
          </List.Item>
        );
      }}
    />
  );
}
