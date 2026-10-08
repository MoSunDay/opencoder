import { AimOutlined, ExpandOutlined, FullscreenExitOutlined, FullscreenOutlined, MinusOutlined, PlusOutlined, SettingOutlined } from "@ant-design/icons";
import { Button, Dropdown, Grid, Segmented, Select, Space, Tooltip, Typography } from "antd";
import type { GraphData } from "../../../types";

type Props = {
  data: GraphData; centerIds: string[]; shownNodes: number; shownEdges: number;
  zoom: number; expanded: boolean; view: string; listOnly: boolean;
  onView: (view: string) => void; onMore: () => void;
  onFocus: (id: string) => void;
  onZoom: (action: "in" | "out" | "reset" | "fit") => void; onAutoFrame: () => void; onExpand: () => void;
};
export default function GraphToolbar({ data, centerIds, shownNodes, shownEdges, zoom, expanded, view, listOnly,
  onView, onMore, onFocus, onZoom, onAutoFrame, onExpand }: Props) {
  const screens = Grid.useBreakpoint();
  const centers = data.nodes.filter((node) => centerIds.includes(node.id));
  const remaining = shownNodes < data.nodes.length || shownEdges < data.edges.length;
  const iconButton = (title: string, icon: React.ReactNode, click: () => void, disabled = !data.nodes.length) =>
    <Tooltip title={title}><Button aria-label={title} icon={icon} onClick={click} disabled={disabled} /></Tooltip>;
  const summary = view === "列表" ? `完整结果：${data.nodes.length} 个实体 · ${data.edges.length} 条关系`
    : `已展示 ${shownNodes}/${data.nodes.length} 个实体 · ${shownEdges}/${data.edges.length} 条关系`;
  return <div className="graph-toolbar">
    <div className="graph-result-summary">
      <Typography.Text type="secondary" ellipsis={{ tooltip: summary }}>{summary}</Typography.Text>
      <Space size={4}>
        {remaining && !listOnly && view === "图" ? <Button size="small" onClick={onMore}>继续展开</Button> : null}
        <Segmented size="small" aria-label="观测结果展示方式" options={[{ label: "图", value: "图", disabled: listOnly }, "列表"]} value={view} onChange={onView} />
      </Space>
    </div>
    {view === "图" ? <div className="graph-toolbar-actions">
      <Select aria-label="在完整结果中搜索节点" placeholder="在结果中查找" allowClear showSearch optionFilterProp="label"
        value={null} className="graph-node-search" disabled={!data.nodes.length} onChange={(id) => { if (id) onFocus(id); }}
        options={data.nodes.map((node) => ({ value: node.id, label: node.name }))} />
      {screens.md ? <Dropdown disabled={!centers.length} menu={{ items: centers.map((node) => ({ key: node.id, label: node.name })), onClick: ({ key }) => onFocus(key) }}>
        <Tooltip title="定位观测中心"><Button aria-label="定位观测中心" icon={<AimOutlined />} disabled={!centers.length} /></Tooltip>
      </Dropdown> : null}
      {screens.md ? iconButton("缩小", <MinusOutlined />, () => onZoom("out")) : null}
      {screens.md ? <Tooltip title="恢复 100%"><Button aria-label="恢复 100%" onClick={() => onZoom("reset")} disabled={!data.nodes.length}>{Math.round(zoom * 100)}%</Button></Tooltip> : null}
      {screens.md ? iconButton("放大", <PlusOutlined />, () => onZoom("in")) : null}
      {iconButton("自动取景", <ExpandOutlined />, onAutoFrame)}
      <Dropdown menu={{ items: [
        { key: "fit", label: "查看全图", onClick: () => onZoom("fit") },
        ...(!screens.md ? [{ key: "in", label: "放大", onClick: () => onZoom("in") }, { key: "out", label: "缩小", onClick: () => onZoom("out") },
          { key: "reset", label: `恢复 100%（当前 ${Math.round(zoom * 100)}%）`, onClick: () => onZoom("reset") }] : []),
      ] }}><Tooltip title="显示设置"><Button aria-label="显示设置" icon={<SettingOutlined />} /></Tooltip></Dropdown>
      {iconButton(expanded ? "恢复画布" : "放大画布", expanded ? <FullscreenExitOutlined /> : <FullscreenOutlined />, onExpand, false)}
    </div> : null}
  </div>;
}
