import { forwardRef } from "react";
import { KIND_DIR } from "./entryModel";
import type { DisplayRow, EntrySortState, SortColumn } from "./entryModel";
import {
  formatBytes,
  formatCount,
  formatModifiedAt,
} from "../../utils/formatLibrarium";
import styles from "./FileTree.module.scss";

interface FileTreeProps {
  dirPath: string;
  rows: DisplayRow[];
  selectedIndex: number;
  focusPane: "tree" | "preview";
  listEmpty: boolean;
  sortState: EntrySortState | null;
  onSortColumn: (column: SortColumn) => void;
  onSelectIndex: (i: number) => void;
  onActivateRow: (row: DisplayRow) => void;
}

function headerLabel(
  column: SortColumn,
  title: string,
  sortState: EntrySortState | null,
): string {
  if (!sortState || sortState.column !== column) {
    return title;
  }
  const arrow = sortState.direction === "asc" ? "↑" : "↓";
  return `${title} ${arrow}`;
}

export const FileTree = forwardRef<HTMLUListElement, FileTreeProps>(
  function FileTree(
    {
      dirPath,
      rows,
      selectedIndex,
      focusPane,
      listEmpty,
      sortState,
      onSortColumn,
      onSelectIndex,
      onActivateRow,
    },
    ref,
  ) {
    return (
      <div className={styles.wrap} data-testid="entries-pane">
        <div className={styles.pathRow} data-testid="entries-path">
          {dirPath}
        </div>
        <div className={styles.headerRow} role="row" aria-label="Sort columns">
          <span className={styles.headerGlyph} aria-hidden />
          <button
            type="button"
            className={styles.headerCell}
            data-testid="sort-name"
            aria-pressed={sortState?.column === "name" ? true : undefined}
            onClick={() => {
              onSortColumn("name");
            }}
          >
            {headerLabel("name", "Name", sortState)}
          </button>
          <button
            type="button"
            className={styles.headerCell}
            data-testid="sort-size"
            aria-pressed={sortState?.column === "size" ? true : undefined}
            onClick={() => {
              onSortColumn("size");
            }}
          >
            {headerLabel("size", "Size", sortState)}
          </button>
          <button
            type="button"
            className={styles.headerCell}
            data-testid="sort-modified"
            aria-pressed={sortState?.column === "modified" ? true : undefined}
            onClick={() => {
              onSortColumn("modified");
            }}
          >
            {headerLabel("modified", "Modified", sortState)}
          </button>
        </div>
        <ul
          ref={ref}
          className={styles.list}
          role="listbox"
          aria-label="Directory entries"
          tabIndex={-1}
        >
          {rows.map((row, i) => {
            const selected = i === selectedIndex;
            if (row.kind === "nav") {
              const label = row.nav === "root" ? "/" : "..";
              const glyph = row.nav === "root" ? "⌂" : "↑";
              return (
                <li
                  key={row.nav === "root" ? "nav-root" : "nav-parent"}
                  role="option"
                  aria-selected={selected}
                  data-nav={row.nav}
                  data-testid={row.nav === "root" ? "nav-root" : "nav-parent"}
                  data-selected={selected ? "true" : "false"}
                  className={`${styles.row} ${styles.rowNav} ${selected ? styles.rowSelected : ""} ${
                    selected && focusPane === "tree" ? styles.rowFocused : ""
                  }`}
                  onClick={() => {
                    onSelectIndex(i);
                    onActivateRow(row);
                  }}
                >
                  <span className={styles.glyph} aria-hidden>
                    {glyph}
                  </span>
                  <span className={styles.nameBlock}>
                    <span className={`${styles.name} ${styles.nameNav}`}>
                      {label}
                    </span>
                  </span>
                  <span className={styles.sizeCol} />
                  <span className={styles.modCol} />
                </li>
              );
            }

            const e = row.entry;
            const isDir = e.kind === KIND_DIR;
            const glyph = isDir ? "▸" : "·";
            const desc = e.description?.trim();
            const modified = formatModifiedAt(e.modified_at);
            const sizeCell = isDir
              ? e.recursive_file_count != null
                ? `${formatCount(e.recursive_file_count)} files`
                : ""
              : formatBytes(e.size_bytes);

            return (
              <li
                key={`${e.kind}-${e.name}`}
                role="option"
                aria-selected={selected}
                data-entry-name={e.name}
                data-entry-kind={isDir ? "dir" : "file"}
                data-selected={selected ? "true" : "false"}
                data-testid={`entry-row-${i}`}
                className={`${styles.row} ${selected ? styles.rowSelected : ""} ${
                  selected && focusPane === "tree" ? styles.rowFocused : ""
                }`}
                onClick={() => {
                  onSelectIndex(i);
                  onActivateRow(row);
                }}
              >
                <span className={styles.glyph} aria-hidden>
                  {glyph}
                </span>
                <span className={styles.nameBlock}>
                  <span
                    className={`${styles.name} ${isDir ? styles.nameDir : styles.nameFile}`}
                  >
                    {e.name}
                  </span>
                  {desc ? (
                    <span className={styles.desc} title={desc}>
                      {desc}
                    </span>
                  ) : null}
                </span>
                <span className={styles.sizeCol}>{sizeCell}</span>
                <span className={styles.modCol}>{modified}</span>
              </li>
            );
          })}
        </ul>
        {listEmpty ? (
          <p className={styles.pathRow}>
            Empty directory — silence in the stacks.
          </p>
        ) : null}
      </div>
    );
  },
);
