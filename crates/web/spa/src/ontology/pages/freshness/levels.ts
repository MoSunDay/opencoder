import type { DataLevel } from "../../types";

/// Data-level ladder, weakest first. Entities enter as "draft"; approving a
/// freshness candidate promotes an entity exactly one step.
export const LEVELS: DataLevel[] = ["draft", "stable", "verified", "confident"];

const LABELS: Record<DataLevel, string> = { draft: "草稿", stable: "稳定", verified: "已验证", confident: "可信" };
const COLORS: Record<DataLevel, string> = { draft: "default", stable: "blue", verified: "green", confident: "gold" };

export function levelLabel(level: DataLevel) {
  return LABELS[level] ?? level;
}

export function levelColor(level: DataLevel) {
  return COLORS[level] ?? "default";
}

export function levelRank(level: DataLevel) {
  const rank = LEVELS.indexOf(level);
  return rank === -1 ? 0 : rank;
}

export function nextLevel(level: DataLevel) {
  return LEVELS[levelRank(level) + 1] ?? null;
}
