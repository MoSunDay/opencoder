import { Button, Card, Input, Space, Tabs, Typography } from "antd";
import ReactMarkdown from "react-markdown";
import type { AttributeDefinition } from "../../types";

export type TextAttribute = {
  definition: AttributeDefinition;
  current?: { revision?: number; bytes?: number };
};

type Props = {
  items: TextAttribute[];
  values: Record<string, string>;
  bodies: Record<string, string>;
  canManage: boolean;
  onChange: (id: string, value: string) => void;
  onSave: (item: TextAttribute) => Promise<void>;
};

export default function TextAttributeSection({ items, values, bodies, canManage, onChange, onSave }: Props) {
  return <>{items.map((item) => {
    const id = item.definition.id;
    const value = values[id] ?? "";
    const nfs = item.definition.storage_mode === "nfs_path";
    return <Space orientation="vertical" key={id} style={{ width: "100%", marginTop: 16 }}>
      <Typography.Text strong>{item.definition.name}</Typography.Text>
      {nfs
        ? <><Input disabled={!canManage} placeholder="受控相对 NFS path" value={value}
          onChange={(event) => onChange(id, event.target.value)} />
          <Card><Typography.Paragraph style={{ whiteSpace: "pre-wrap" }}>{bodies[id] ?? ""}</Typography.Paragraph></Card></>
        : <Tabs items={[
          { key: "edit", label: "编辑", children: <Input.TextArea disabled={!canManage} rows={5} value={value}
            onChange={(event) => onChange(id, event.target.value)} /> },
          { key: "preview", label: "预览", children: <Card><ReactMarkdown skipHtml>{value}</ReactMarkdown></Card> },
        ]} />}
      {canManage ? <Button type="primary" onClick={() => void onSave(item)}>保存</Button> : null}
    </Space>;
  })}</>;
}
