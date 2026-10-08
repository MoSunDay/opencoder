export const RELATIONSHIP_FONT_SIZE = 13;
export const RELATIONSHIP_LINE_HEIGHT = 20;
export const RELATIONSHIP_FONT_FAMILY = "system-ui, sans-serif";
const MAX_TEXT_WIDTH = 180;
const PADDING = 6;

/** Wrap without discarding characters; the same dimensions reserve space in Dagre. */
export function relationshipLabel(name: string, measure: (text: string) => number) {
  const lines: string[] = [];
  for (const paragraph of name.split("\n")) {
    let line = "";
    for (const character of Array.from(paragraph)) {
      if (line && measure(line + character) > MAX_TEXT_WIDTH) { lines.push(line); line = ""; }
      line += character;
    }
    lines.push(line);
  }
  return {
    text: lines.join("\n"),
    width: Math.ceil(Math.max(...lines.map(measure))) + PADDING * 2,
    height: lines.length * RELATIONSHIP_LINE_HEIGHT + PADDING * 2,
  };
}
