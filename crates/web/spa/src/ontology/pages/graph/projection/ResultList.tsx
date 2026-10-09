import InspectLink from "../../../navigation/InspectLink";
import { Button, Input, Segmented, Space, Table, Tag } from "antd";
import { useMemo, useState } from "react";
import type { GraphData } from "../../../types";

type Props = { data: GraphData; entityTypeNames: Record<string, string>; relationshipTypeNames: Record<string, string>;
  shownIds: string[]; onNodeClick: (id: string) => void; onEdgeClick: (id: string) => void; onLocate: (id: string) => void };
export default function ResultList({ data, entityTypeNames, relationshipTypeNames, shownIds, onNodeClick, onEdgeClick, onLocate }: Props) {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState("实体");
  const [page, setPage] = useState(1);
  const shown = new Set(shownIds);
  const names = useMemo(() => Object.fromEntries(data.nodes.map((node) => [node.id, node.name])), [data]);
  const match = (values: string[]) => values.join(" ").toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  return <div className="graph-result-list">
    <Space wrap style={{ marginBottom: 12 }}>
      <Segmented options={["实体", "关系"]} value={mode} onChange={(value) => { setMode(value); setPage(1); }} />
      <Input.Search aria-label="搜索完整观测结果" placeholder="搜索名称、类型或 ID" allowClear value={query} onChange={(e) => { setQuery(e.target.value); setPage(1); }} />
    </Space>
    {mode === "实体" ? <Table rowKey="id" size="small" scroll={{ x: 600 }} pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }}
      dataSource={data.nodes.filter((node) => match([node.name, node.id, entityTypeNames[node.entity_type_id] ?? ""]))}
      columns={[
        { title: "实体", dataIndex: "name", width: 260, render: (name, node) => <InspectLink onClick={() => onNodeClick(node.id)}>{name}</InspectLink> },
        { title: "类型", render: (_, node) => entityTypeNames[node.entity_type_id] },
        { title: "画布", render: (_, node) => shown.has(node.id) ? <Tag>已展示</Tag> : <Tag color="gold">尚未展示</Tag> },
        { title: "操作", width: 100, render: (_, node) => <Button type="link" onClick={() => onLocate(node.id)}>在图中定位</Button> },
      ]} /> : <Table rowKey="id" size="small" scroll={{ x: 600 }} pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }}
      dataSource={data.edges.filter((edge) => match([names[edge.source_entity_id] ?? "", names[edge.target_entity_id] ?? "", relationshipTypeNames[edge.relationship_type_id] ?? "", edge.id]))}
      columns={[
        { title: "源实体", render: (_, edge) => <InspectLink onClick={() => onNodeClick(edge.source_entity_id)}>{names[edge.source_entity_id]}</InspectLink> },
        { title: "关系 →", render: (_, edge) => <InspectLink onClick={() => onEdgeClick(edge.id)}>{relationshipTypeNames[edge.relationship_type_id]}</InspectLink> },
        { title: "目标实体", render: (_, edge) => <InspectLink onClick={() => onNodeClick(edge.target_entity_id)}>{names[edge.target_entity_id]}</InspectLink> },
      ]} />}
  </div>;
}
