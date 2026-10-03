export type StructuredAttributeRow = { attribute_definition_id: string; value: unknown; revision: number };
export function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}
