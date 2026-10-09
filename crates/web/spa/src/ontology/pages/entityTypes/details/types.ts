import type { AttributeDefinition } from "../../../types";

export type TextFormat = "md" | "html" | "nfs_path";
export type TextAttribute = {
  definition: AttributeDefinition;
  current?: { revision?: number; bytes?: number; content_path?: string; format?: string };
};
export type StructuredAttributeRow = {
  attribute_definition_id: string;
  value: unknown;
  revision: number;
  is_deleted?: boolean;
};
export type StructuredDraft = { value: unknown; original: unknown; revision: number };
