import type { Content, Element, Parent, Root, Text } from "hast";

export const PREVIEW_HIGHLIGHT_CLASS = "previewHighlight";

type Segment = { kind: "text" | "mark"; value: string };

/** Terms for matching: full trimmed query plus whitespace-separated tokens (min length 2). Longest first. */
export function extractSearchHighlightTerms(query: string): string[] {
  const raw = query.trim();
  if (raw.length < 2) {
    return [];
  }
  const seen = new Set<string>();
  const out: string[] = [];
  const add = (s: string): void => {
    const t = s.trim();
    if (t.length < 2) {
      return;
    }
    const k = t.toLowerCase();
    if (seen.has(k)) {
      return;
    }
    seen.add(k);
    out.push(t);
  };
  add(raw);
  for (const p of raw.split(/\s+/)) {
    add(p);
  }
  out.sort((a, b) => b.length - a.length);
  return out;
}

function splitTextByTerms(text: string, terms: string[]): Segment[] {
  if (terms.length === 0 || text.length === 0) {
    return [{ kind: "text", value: text }];
  }
  const lower = text.toLowerCase();
  let best: { start: number; end: number } | null = null;
  for (const term of terms) {
    const tl = term.toLowerCase();
    let from = 0;
    while (from < text.length) {
      const i = lower.indexOf(tl, from);
      if (i < 0) {
        break;
      }
      const end = i + term.length;
      if (
        best === null ||
        i < best.start ||
        (i === best.start && end - i > best.end - best.start)
      ) {
        best = { start: i, end };
      }
      from = i + 1;
    }
  }
  if (best === null) {
    return [{ kind: "text", value: text }];
  }
  const out: Segment[] = [];
  if (best.start > 0) {
    out.push(...splitTextByTerms(text.slice(0, best.start), terms));
  }
  out.push({ kind: "mark", value: text.slice(best.start, best.end) });
  if (best.end < text.length) {
    out.push(...splitTextByTerms(text.slice(best.end), terms));
  }
  return out;
}

function segmentsToHast(segments: Segment[]): Content[] {
  const out: Content[] = [];
  for (const s of segments) {
    if (s.kind === "text") {
      out.push({ type: "text", value: s.value } satisfies Text);
    } else {
      const el: Element = {
        type: "element",
        tagName: "mark",
        properties: { className: [PREVIEW_HIGHLIGHT_CLASS] },
        children: [{ type: "text", value: s.value }],
      };
      out.push(el);
    }
  }
  return out;
}

function transformChildren(parent: Parent, terms: string[]): void {
  let i = 0;
  while (i < parent.children.length) {
    const node = parent.children[i];
    if (node.type === "element") {
      const tag = node.tagName;
      if (tag === "code" || tag === "pre" || tag === "mark") {
        i += 1;
        continue;
      }
      transformChildren(node, terms);
      i += 1;
      continue;
    }
    if (node.type === "text") {
      const segs = splitTextByTerms(node.value, terms);
      const hastNodes = segmentsToHast(segs);
      if (
        hastNodes.length === 1 &&
        hastNodes[0].type === "text" &&
        hastNodes[0].value === node.value
      ) {
        i += 1;
        continue;
      }
      parent.children.splice(i, 1, ...hastNodes);
      i += hastNodes.length;
      continue;
    }
    i += 1;
  }
}

/** Mutates HAST from react-markdown / rehype — highlight plain text (skips code/pre/mark subtrees). */
export function applySearchHighlightToHast(root: Root, terms: string[]): void {
  if (terms.length === 0) {
    return;
  }
  transformChildren(root, terms);
}

export function splitPlainTextForPreview(
  text: string,
  terms: string[],
): Segment[] {
  return splitTextByTerms(text, terms);
}

/** Rehype plugin attacher (run before `rehype-sanitize` with a schema that allows `mark`). */
export function rehypeSearchHighlight(
  query: string,
): () => (tree: Root) => void {
  const terms = extractSearchHighlightTerms(query);
  return function attacher() {
    return (tree: Root) => {
      applySearchHighlightToHast(tree, terms);
    };
  };
}
