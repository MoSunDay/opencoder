import { Tag } from "antd";
import type { DataLevel } from "../../types";
import { levelColor, levelLabel } from "./levels";

export default function LevelBadge({ level = "draft" }: { level?: DataLevel }) {
  return <Tag color={levelColor(level)}>{levelLabel(level)}</Tag>;
}
