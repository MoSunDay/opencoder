export type LabelBox = { x: number; y: number; width: number; height: number };

/** Keep complete labels inside the viewport and separate them where space permits. */
export function placeLabel(anchor: { x: number; y: number }, width: number, height: number,
  occupied: LabelBox[], viewport: { width: number; height: number }): LabelBox {
  let best: LabelBox = { x: anchor.x - width / 2, y: anchor.y - height / 2, width, height };
  let bestScore = Infinity;
  for (const radius of [0, 24, 48, 80, 120]) for (let step = 0; step < (radius ? 8 : 1); step++) {
    const angle = step * Math.PI / 4;
    const box = { x: Math.max(4, Math.min(viewport.width - width - 4, anchor.x + Math.cos(angle) * radius - width / 2)),
      y: Math.max(4, Math.min(viewport.height - height - 4, anchor.y + Math.sin(angle) * radius - height / 2)), width, height };
    const overlap = occupied.reduce((sum, other) => sum + Math.max(0, Math.min(box.x + width + 4, other.x + other.width) - Math.max(box.x - 4, other.x))
      * Math.max(0, Math.min(box.y + height + 4, other.y + other.height) - Math.max(box.y - 4, other.y)), 0);
    const score = overlap * 1000 + Math.hypot(box.x + width / 2 - anchor.x, box.y + height / 2 - anchor.y);
    if (score < bestScore) { best = box; bestScore = score; }
  }
  return best;
}
