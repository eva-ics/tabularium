import type { Schema } from "hast-util-sanitize";
import { defaultSchema } from "hast-util-sanitize";

const GFM_TABLE_TAGS = ["table", "thead", "tbody", "tr", "th", "td"] as const;

/** Merge GFM table element names into a rehype-sanitize schema. */
export function extendSchemaWithGfmTables(base: Schema): Schema {
  return {
    ...base,
    tagNames: [...new Set([...(base.tagNames ?? []), ...GFM_TABLE_TAGS])],
  };
}

export const gfmTableSanitizeSchema = extendSchemaWithGfmTables(defaultSchema);
