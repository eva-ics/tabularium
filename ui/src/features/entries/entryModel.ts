import type { ListedEntry } from "../../api/client";

export const KIND_DIR = 0;
export const KIND_FILE = 1;

export type SortColumn = "name" | "size" | "modified";

export type SortDirection = "asc" | "desc";

export interface EntrySortState {
  column: SortColumn;
  direction: SortDirection;
}

export function parentPath(p: string): string {
  const t = p.replace(/\/+$/, "") || "/";
  if (t === "/") {
    return "/";
  }
  const i = t.lastIndexOf("/");
  if (i <= 0) {
    return "/";
  }
  return t.slice(0, i);
}

export function sortedEntries(entries: ListedEntry[]): ListedEntry[] {
  return [...entries].sort((a, b) => {
    if (a.kind !== b.kind) {
      return a.kind - b.kind;
    }
    return a.name.localeCompare(b.name);
  });
}

function sortKeySize(e: ListedEntry): number {
  if (e.kind === KIND_DIR) {
    return e.recursive_file_count ?? -1;
  }
  return e.size_bytes ?? -1;
}

function sortKeyModified(e: ListedEntry): number {
  return e.modified_at ?? 0;
}

/** Default dirs-first + name order until the user picks a column; then stable UI-side sort. */
export function orderEntriesForDisplay(
  entries: ListedEntry[],
  userSort: EntrySortState | null,
): ListedEntry[] {
  if (userSort === null) {
    return sortedEntries(entries);
  }
  const mul = userSort.direction === "asc" ? 1 : -1;
  const tagged = entries.map((e, i) => ({ e, i }));
  tagged.sort((a, b) => {
    let c = 0;
    switch (userSort.column) {
      case "name":
        c = a.e.name.localeCompare(b.e.name);
        break;
      case "size":
        c = sortKeySize(a.e) - sortKeySize(b.e);
        break;
      case "modified":
        c = sortKeyModified(a.e) - sortKeyModified(b.e);
        break;
    }
    if (c !== 0) {
      return mul * c;
    }
    return a.i - b.i;
  });
  return tagged.map((t) => t.e);
}

/** Synthetic `/` and `..` plus API entries — single view-model for mouse + keyboard (Logis). */
export type DisplayRow =
  | {
      kind: "nav";
      nav: "root" | "parent";
      targetPath: string;
    }
  | { kind: "entry"; entry: ListedEntry };

export function buildDisplayRows(
  dirPath: string,
  orderedEntries: ListedEntry[],
): DisplayRow[] {
  const rows: DisplayRow[] = [{ kind: "nav", nav: "root", targetPath: "/" }];
  if (dirPath !== "/") {
    rows.push({
      kind: "nav",
      nav: "parent",
      targetPath: parentPath(dirPath),
    });
  }
  for (const entry of orderedEntries) {
    rows.push({ kind: "entry", entry });
  }
  return rows;
}
