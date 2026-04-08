import type { Schema } from "hast-util-sanitize";
import { defaultSchema } from "hast-util-sanitize";

const GFM_TABLE_TAGS = ["table", "thead", "tbody", "tr", "th", "td"] as const;

/** KaTeX / MathML tags not present in GitHub-style defaultSchema. */
const KATEX_EXTRA_TAG_NAMES = [
  "annotation",
  "annotation-xml",
  "math",
  "menclose",
  "mfrac",
  "mi",
  "mlabeledtr",
  "mn",
  "mo",
  "mover",
  "mpadded",
  "mroot",
  "mrow",
  "ms",
  "mspace",
  "msqrt",
  "mstyle",
  "msub",
  "msubsup",
  "msup",
  "semantics",
  "mtable",
  "mtd",
  "mtext",
  "mtr",
  "munder",
  "munderover",
  "path",
  "svg",
] as const;

function mergeSchemaAttributes(
  base: NonNullable<Schema["attributes"]>,
  extra: NonNullable<Schema["attributes"]>,
): NonNullable<Schema["attributes"]> {
  const out: NonNullable<Schema["attributes"]> = { ...base };
  for (const [key, val] of Object.entries(extra)) {
    if (val === undefined) {
      continue;
    }
    const prev = out[key as keyof NonNullable<Schema["attributes"]>];
    if (prev === undefined) {
      Object.assign(out, { [key]: val });
    } else if (Array.isArray(prev) && Array.isArray(val)) {
      Object.assign(out, { [key]: [...prev, ...val] });
    } else {
      Object.assign(out, { [key]: val });
    }
  }
  return out;
}

const katexAttributePatch: NonNullable<Schema["attributes"]> = {
  span: [
    "style",
    "ariaHidden",
    // KaTeX uses hyphens (`katex-error`) and digits (`size11`); `-` must be literal, not a range.
    ["className", /^[-a-zA-Z0-9_]+$/],
  ],
  math: ["xmlns"],
  svg: [
    "xmlns",
    "width",
    "height",
    "viewBox",
    "preserveAspectRatio",
    "focusable",
    "ariaHidden",
  ],
  path: ["d"],
  mi: ["mathvariant"],
  annotation: ["encoding"],
  code: [
    ["className", "math-inline"],
    ["className", "math-display"],
  ],
};

/** Allow KaTeX output through `rehype-sanitize` (run sanitize after `rehype-katex`). */
export function extendSchemaWithKatex(base: Schema): Schema {
  return {
    ...base,
    tagNames: [
      ...new Set([...(base.tagNames ?? []), ...KATEX_EXTRA_TAG_NAMES]),
    ],
    attributes: mergeSchemaAttributes(
      base.attributes ?? {},
      katexAttributePatch,
    ),
  };
}

/** Merge GFM table element names into a rehype-sanitize schema. */
export function extendSchemaWithGfmTables(base: Schema): Schema {
  return {
    ...base,
    tagNames: [...new Set([...(base.tagNames ?? []), ...GFM_TABLE_TAGS])],
  };
}

export const gfmTableSanitizeSchema = extendSchemaWithGfmTables(defaultSchema);

export const gfmTableKatexSanitizeSchema = extendSchemaWithKatex(
  gfmTableSanitizeSchema,
);
