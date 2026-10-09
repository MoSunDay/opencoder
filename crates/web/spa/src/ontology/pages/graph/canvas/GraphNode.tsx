import { Tag, Tooltip, Typography } from "antd";
import { nodeTitle } from "./presentation";

type Props = {
  name: string;
  typeName: string;
  center: boolean;
  onActivate?: () => void;
  onExpand?: () => void;
  remaining?: number;
  isSelected?: boolean;
  isActive?: boolean;
  colors: { background: string; text: string; secondary: string; border: string; primary: string; selected: string };
};

export default function GraphNode({ name, typeName, center, isSelected, isActive, colors, onActivate, onExpand, remaining = 0 }: Props) {
  const palette = ["blue", "cyan", "purple", "geekblue", "gold", "green"];
  const color = palette[[...typeName].reduce((sum, character) => sum + character.charCodeAt(0), 0) % palette.length];
  const label = nodeTitle(name);
  return <div className="graph-node" role="button" tabIndex={0} aria-label={`${typeName}：${name}${center ? "，观测中心" : ""}`}
    onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onActivate?.(); } }} style={{ color: colors.text,
    background: isSelected ? colors.selected : colors.background,
    borderColor: center || isSelected || isActive ? colors.primary : colors.border,
    boxShadow: isSelected ? `0 0 0 2px ${colors.selected}` : undefined }}>
    <div className="graph-node-type" style={{ color: colors.secondary }}>
      <Tag color={color} style={{ maxWidth: center ? 125 : 196, overflow: "hidden", textOverflow: "ellipsis", margin: 0 }}>{typeName}</Tag>
      {center ? <Tag color="blue" style={{ margin: 0, fontSize: 11 }}>中心</Tag> : null}
    </div>
    <Tooltip title={name}><div className="graph-node-title">{label.detail ? <><strong>{label.title}</strong><br /><span>{label.detail}</span></> : label.title}</div></Tooltip>
    {remaining > 0 ? <Typography.Link role="button" tabIndex={0} className="graph-node-expand" onClick={(event) => { event.stopPropagation(); onExpand?.(); }}
      onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); onExpand?.(); } }}>还有 {remaining} 个关联 · 展开</Typography.Link> : null}
  </div>;
}
