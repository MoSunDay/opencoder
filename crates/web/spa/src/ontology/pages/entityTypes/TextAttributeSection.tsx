import { EditOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Empty, Input, Skeleton, Space, Typography } from "antd";
import TextPreview from "./details/TextPreview";
import type { TextAttribute } from "./details/types";
import type { useTextDrafts } from "./details/useTextDrafts";
import "./details/content.css";

type Props = { items: TextAttribute[]; canManage: boolean; drafts: ReturnType<typeof useTextDrafts> };

export default function TextAttributeSection({ items, canManage, drafts }: Props) {
  return <Space direction="vertical" size="middle" style={{ width: "100%" }}>{items.map((item) => {
    const id = item.definition.id;
    const value = drafts.values[id] ?? "";
    const nfs = item.definition.storage_mode === "nfs_path";
    const editing = drafts.editing[id];
    const ready = Object.prototype.hasOwnProperty.call(drafts.values, id);
    return <Card key={id} size="small" title={item.definition.name}
      extra={canManage && !editing ? <Button size="small" icon={<EditOutlined />} disabled={!ready || drafts.loading[id]}
        onClick={() => drafts.edit(id)}>编辑内容</Button> : null}>
      {drafts.errors[id] ? <Alert type="error" showIcon message={drafts.errors[id]} style={{ marginBottom: 12 }}
        action={!ready || !editing ? <Button onClick={() => drafts.retry(item)}>重试</Button> : <Button onClick={() => void drafts.refresh()}>刷新版本</Button>} /> : null}
      {drafts.loading[id] || (!ready && !drafts.errors[id]) ? <Skeleton active paragraph={{ rows: 3 }} />
        : editing ? <Space direction="vertical" style={{ width: "100%" }}>
          {nfs ? <Input aria-label={`${item.definition.name}路径`} placeholder="受控相对路径" value={value} disabled={drafts.saving[id]}
            onChange={(event) => drafts.change(id, event.target.value)} />
            : <Input.TextArea aria-label={`${item.definition.name}正文`} autoSize={{ minRows: 8, maxRows: 20 }} value={value} disabled={drafts.saving[id]}
              onChange={(event) => drafts.change(id, event.target.value)} />}
          <Space className="entity-editor-actions"><Button type="primary" loading={drafts.saving[id]} onClick={() => void drafts.save(item)}>保存</Button>
            <Button disabled={drafts.saving[id]} onClick={() => drafts.cancel(id)}>取消</Button></Space>
        </Space> : ready ? <div className="entity-text-preview">
          {nfs ? <><Typography.Paragraph copyable>{value || "尚未填写路径"}</Typography.Paragraph>
            <Typography.Paragraph style={{ whiteSpace: "pre-wrap" }}>{drafts.bodies[id]}</Typography.Paragraph></>
            : value ? <TextPreview name={item.definition.name} content={value} format={drafts.formats[id]} /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未填写内容" />}
        </div> : null}
    </Card>;
  })}</Space>;
}
