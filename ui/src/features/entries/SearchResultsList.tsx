import { forwardRef, type KeyboardEvent } from "react";
import type { SearchHit } from "../../api/client";
import styles from "./SearchResultsList.module.scss";

export type SearchListRow =
  | { kind: "nav"; nav: "parent" | "root" }
  | { kind: "hit"; hit: SearchHit };

interface SearchResultsListProps {
  /** Shown in `entries-path` (truncated): "Search results for: …". */
  resultsHeading: string;
  rows: SearchListRow[];
  selectedIndex: number;
  focusTree: boolean;
  loading: boolean;
  error: string | null;
  onSelectIndex: (i: number) => void;
  onActivate: (row: SearchListRow) => void;
  onKeyDown?: (e: KeyboardEvent<HTMLUListElement>) => void;
}

export const SearchResultsList = forwardRef<
  HTMLUListElement,
  SearchResultsListProps
>(function SearchResultsList(
  {
    resultsHeading,
    rows,
    selectedIndex,
    focusTree,
    loading,
    error,
    onSelectIndex,
    onActivate,
    onKeyDown,
  },
  ref,
) {
  return (
    <div className={styles.wrap} data-testid="search-mode">
      <div
        className={styles.pathRow}
        data-testid="entries-path"
        title={resultsHeading}
      >
        <span className={styles.resultsHeading}>{resultsHeading}</span>
      </div>
      {error ? (
        <p className={styles.pathRow} role="alert">
          {error}
        </p>
      ) : null}
      {loading ? <p className={styles.pathRow}>Searching the stacks…</p> : null}
      <ul
        ref={ref}
        className={styles.list}
        role="listbox"
        tabIndex={-1}
        aria-label="Search results"
        onKeyDown={onKeyDown}
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
                data-testid={row.nav === "root" ? "nav-root" : "nav-parent"}
                data-selected={selected ? "true" : "false"}
                className={`${styles.row} ${styles.rowNav} ${selected ? styles.rowSelected : ""} ${
                  selected && focusTree ? styles.rowFocused : ""
                }`}
                onClick={() => {
                  onSelectIndex(i);
                  onActivate(row);
                }}
              >
                <span className={styles.glyph} aria-hidden>
                  {glyph}
                </span>
                <div className={styles.mainCol}>
                  <span className={styles.nameNav}>{label}</span>
                </div>
              </li>
            );
          }
          const h = row.hit;
          return (
            <li
              key={`${h.path}-${h.document_id}`}
              role="option"
              aria-selected={selected}
              data-testid="search-result-row"
              data-result-path={h.path}
              data-result-score={String(h.score)}
              data-selected={selected ? "true" : "false"}
              className={`${styles.row} ${selected ? styles.rowSelected : ""} ${
                selected && focusTree ? styles.rowFocused : ""
              }`}
              onClick={() => {
                onSelectIndex(i);
                onActivate(row);
              }}
            >
              <span className={styles.glyph} aria-hidden>
                ·
              </span>
              <div className={styles.mainCol}>
                <span className={styles.path}>{h.path}</span>
                <span className={styles.score}>{h.score.toFixed(4)}</span>
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
});
