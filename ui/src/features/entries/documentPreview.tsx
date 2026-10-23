import { createElement, type ReactNode } from "react";
import type { RootContent } from "hast";
import { createLowlight } from "lowlight";
import json from "highlight.js/lib/languages/json";
import bash from "highlight.js/lib/languages/bash";
import {
  PREVIEW_HIGHLIGHT_CLASS,
  splitPlainTextForPreview,
} from "./highlightSearchTerms";

const lowlight = createLowlight({ json, bash });
export type PreviewFormat = "markdown" | "json" | "bash" | "text";

/** Explicit MIME wins; extension inference is only for missing/empty metadata. */
export function documentPreviewFormat(path: string | null, mime?: string | null): PreviewFormat {
  const type = mime?.split(";", 1)[0].trim().toLowerCase();
  if (type) {
    if (type === "text/plain") return "text";
    if (type === "application/json" || type === "text/json" || type.endsWith("+json")) return "json";
    if (["application/x-sh", "application/x-shellscript", "text/x-shellscript", "text/x-sh", "text/x-bash", "application/x-bash"].includes(type)) return "bash";
    return "markdown";
  }
  const name = path?.split("/").pop()?.toLowerCase() ?? "";
  if (name.endsWith(".txt") || name.endsWith(".list")) return "text";
  if (name.endsWith(".json")) return "json";
  if (name.endsWith(".sh") || name.endsWith(".bash")) return "bash";
  return "markdown";
}

/** Render a token tree as React text/spans, never executable document HTML. */
export function codePreviewNodes(content: string, language: "json" | "bash" | "text", terms: string[]): ReactNode {
  function text(value: string, key: string): ReactNode {
    if (terms.length === 0) return value;
    return splitPlainTextForPreview(value, terms).map((part, i) => part.kind === "text"
      ? part.value
      : createElement("mark", { key: `${key}-${i}`, className: PREVIEW_HIGHLIGHT_CLASS }, part.value));
  }
  function node(part: RootContent, key: string): ReactNode {
    if (part.type === "text") return text(part.value, key);
    if (part.type !== "element") return null;
    const classes = Array.isArray(part.properties.className) ? part.properties.className.join(" ") : undefined;
    return createElement("span", { key, className: classes }, part.children.map((child, i) => node(child, `${key}-${i}`)));
  }
  // Keep large documents responsive; plain text still preserves all source bytes.
  if (language === "text" || content.length > 256_000) return text(content, "plain");
  try {
    return lowlight.highlight(language, content).children.map((part, i) => node(part, String(i)));
  } catch {
    return text(content, "plain");
  }
}
