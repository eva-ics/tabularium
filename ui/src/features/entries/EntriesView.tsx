import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent,
} from "react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import {
  type ListedEntry,
  getDocument,
  listDirectory,
  searchDocuments,
  type SearchHit,
} from "../../api/client";
import { useAppShell } from "../../app/appShellContext";
import { useLibraryKeyboard } from "../../hooks/useLibraryKeyboard";
import {
  KIND_DIR,
  KIND_FILE,
  buildDisplayRows,
  orderEntriesForDisplay,
  parentPath,
  type DisplayRow,
  type EntrySortState,
  type SortColumn,
} from "./entryModel";
import {
  dirPathFromEntriesLocation,
  entriesPathForDir,
  withOpenDocQuery,
} from "./entriesPath";
import { FileTree } from "./FileTree";
import { PREVIEW_HIGHLIGHT_CLASS } from "./highlightSearchTerms";
import {
  PreviewPane,
  type DocumentSurface,
  type EditSessionState,
} from "./PreviewPane";
import { SearchResultsList, type SearchListRow } from "./SearchResultsList";
import styles from "./EntriesView.module.scss";

const OPEN_Q = "open";

const ENTRIES_DELEGATE_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "Home",
  "End",
  "PageUp",
  "PageDown",
]);

function childPath(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}

const PREVIEW_SCROLL_STEP = 48;

function firstDataEntryIndex(rows: DisplayRow[]): number {
  const i = rows.findIndex((r) => r.kind === "entry");
  return i >= 0 ? i : 0;
}

function scrollListSelectionIntoView(
  listEl: HTMLUListElement | null,
  index: number,
): void {
  if (!listEl) {
    return;
  }
  const child = listEl.children.item(index);
  if (!(child instanceof HTMLElement)) {
    return;
  }
  child.scrollIntoView({ block: "nearest", inline: "nearest" });
}

function pageMoveIndex(
  listEl: HTMLUListElement | null,
  currentIndex: number,
  dir: 1 | -1,
): number {
  if (!listEl) {
    return currentIndex;
  }
  const children = [...listEl.children].filter(
    (child): child is HTMLElement => child instanceof HTMLElement,
  );
  if (children.length === 0) {
    return 0;
  }
  const clampedCurrent = Math.min(
    Math.max(0, currentIndex),
    children.length - 1,
  );
  const currentEl = children[clampedCurrent];
  const step = Math.max(1, Math.round(listEl.clientHeight * 0.85));
  const targetTop = currentEl.offsetTop + dir * step;

  if (dir === 1) {
    for (let i = clampedCurrent + 1; i < children.length; i++) {
      if (children[i].offsetTop >= targetTop) {
        return i;
      }
    }
    return children.length - 1;
  }

  for (let i = clampedCurrent - 1; i >= 0; i--) {
    if (children[i].offsetTop <= targetTop) {
      return i;
    }
  }
  return 0;
}

/**
 * When the preview pane is active, arrow/page keys should scroll the document viewport if
 * focus is on the preview body, or on `document.body` / `<html>` (e.g. Selenium `body.send_keys`),
 * but not on sort buttons or other controls (Ferrum / machine-spirit rites).
 */
function libraryKeysScrollPreviewViewport(
  focusPane: "tree" | "preview",
  previewBody: HTMLDivElement | null,
  previewContent: string | null,
  previewLoading: boolean,
): boolean {
  if (focusPane !== "preview" || previewContent === null || previewLoading) {
    return false;
  }
  if (typeof document === "undefined") {
    return false;
  }
  const ae = document.activeElement;
  if (previewBody != null && ae === previewBody) {
    return true;
  }
  return ae === document.body || ae === document.documentElement;
}

export function EntriesView() {
  const { setAppReady } = useAppShell();
  const location = useLocation();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const previewBodyRef = useRef<HTMLDivElement>(null);
  const pendingFocusName = useRef<string | null>(null);
  const pendingSearchRestore = useRef<{
    selectedIndex: number;
    scrollTop: number;
  } | null>(null);
  const entrySortRef = useRef<EntrySortState | null>(null);
  const listScrollRef = useRef<HTMLUListElement>(null);
  const searchListRef = useRef<HTMLUListElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const snapshotRef = useRef<{
    dirPath: string;
    selectedIndex: number;
    scrollTop: number;
    scope: "local" | "global";
  } | null>(null);
  const searchSessionRef = useRef(false);
  const prevFiredTrim = useRef("");
  const firedQueryRef = useRef("");
  const prevSearchLoading = useRef(false);
  const focusSyncFrame = useRef<number | null>(null);
  const pendingListScrollRestore = useRef<number | null>(null);
  const displayRowsRef = useRef<DisplayRow[]>([]);
  const searchRowsRef = useRef<SearchListRow[]>([]);
  const selectedIndexRef = useRef(0);
  const searchSelectedIndexRef = useRef(0);

  const dirPath = useMemo(
    () => dirPathFromEntriesLocation(location.pathname),
    [location.pathname],
  );

  const openDocPath = searchParams.get(OPEN_Q);

  const editSessionRef = useRef({ active: false, dirty: false });
  const [docReloadNonce, setDocReloadNonce] = useState(0);
  const [documentSurface, setDocumentSurface] =
    useState<DocumentSurface>("preview");

  const [entries, setEntries] = useState<ListedEntry[]>([]);
  const [listError, setListError] = useState<string | null>(null);
  const [loadingList, setLoadingList] = useState(true);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [focusPane, setFocusPane] = useState<"tree" | "preview">("tree");
  const [previewContent, setPreviewContent] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [mobilePanel, setMobilePanel] = useState<"entries" | "preview">(
    "entries",
  );
  const [entrySort, setEntrySort] = useState<EntrySortState | null>(null);
  entrySortRef.current = entrySort;

  const [queryInput, setQueryInput] = useState("");
  /** Committed query — API runs only after Enter (Enginseer heresy A). */
  const [firedQuery, setFiredQuery] = useState("");
  const [searchHits, setSearchHits] = useState<SearchHit[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [searchSelectedIndex, setSearchSelectedIndex] = useState(0);
  const [searchScope, setSearchScope] = useState<"local" | "global">("local");
  const [sessionSearchScope, setSessionSearchScope] = useState<
    "local" | "global" | null
  >(null);
  const [previewHighlightQuery, setPreviewHighlightQuery] = useState<
    string | null
  >(null);

  const orderedEntries = useMemo(
    () => orderEntriesForDisplay(entries, entrySort),
    [entries, entrySort],
  );

  const displayRows = useMemo(
    () => buildDisplayRows(dirPath, orderedEntries),
    [dirPath, orderedEntries],
  );

  const treeRows = useMemo(
    () => (loadingList ? buildDisplayRows(dirPath, []) : displayRows),
    [loadingList, dirPath, displayRows],
  );

  const searchRows = useMemo((): SearchListRow[] => {
    const r: SearchListRow[] = [
      { kind: "nav", nav: "root" },
      { kind: "nav", nav: "parent" },
    ];
    for (const h of searchHits) {
      r.push({ kind: "hit", hit: h });
    }
    return r;
  }, [searchHits]);

  const searchModeUi = firedQuery.trim() !== "";
  firedQueryRef.current = firedQuery;

  displayRowsRef.current = displayRows;
  searchRowsRef.current = searchRows;
  selectedIndexRef.current = selectedIndex;
  searchSelectedIndexRef.current = searchSelectedIndex;

  useEffect(() => {
    if (firedQuery.trim() === "") {
      setSearchHits([]);
      setSearchError(null);
      setSearchLoading(false);
      return;
    }
    let cancelled = false;
    setSearchLoading(true);
    setSearchError(null);
    void (async () => {
      try {
        const snap = snapshotRef.current;
        const subtreeDir = snap?.scope === "local" ? snap.dirPath : undefined;
        const hits = await searchDocuments(firedQuery, { subtreeDir });
        if (!cancelled) {
          setSearchHits(hits);
        }
      } catch (e) {
        if (!cancelled) {
          setSearchError(e instanceof Error ? e.message : String(e));
          setSearchHits([]);
        }
      } finally {
        if (!cancelled) {
          setSearchLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [firedQuery, sessionSearchScope]);

  const confirmDiscardIfNeeded = useCallback((): boolean => {
    const s = editSessionRef.current;
    if (s.active && s.dirty) {
      return window.confirm("Discard unsaved changes?");
    }
    return true;
  }, []);

  const onEditSessionChange = useCallback((state: EditSessionState) => {
    editSessionRef.current = state;
  }, []);

  const bumpOpenDocumentReload = useCallback(() => {
    setDocReloadNonce((n) => n + 1);
  }, []);

  useEffect(() => {
    if (firedQuery.trim() === "") {
      prevSearchLoading.current = searchLoading;
      return;
    }
    if (prevSearchLoading.current && !searchLoading) {
      const idx = searchHits.length > 0 ? 2 : 0;
      setSearchSelectedIndex(idx);
    }
    prevSearchLoading.current = searchLoading;
  }, [searchLoading, firedQuery, searchHits.length]);

  const restoreSnapshot = useCallback(() => {
    const s = snapshotRef.current;
    if (!s) {
      return;
    }
    if (s.dirPath !== dirPath) {
      pendingSearchRestore.current = {
        selectedIndex: s.selectedIndex,
        scrollTop: s.scrollTop,
      };
      navigate(entriesPathForDir(s.dirPath));
    } else {
      setSelectedIndex(s.selectedIndex);
      pendingListScrollRestore.current = s.scrollTop;
    }
  }, [dirPath, navigate]);

  const exitSearchSession = useCallback(() => {
    if (!confirmDiscardIfNeeded()) {
      return;
    }
    searchSessionRef.current = false;
    setFiredQuery("");
    setQueryInput("");
    setSessionSearchScope(null);
    setFocusPane("tree");
    setMobilePanel("entries");
    searchInputRef.current?.blur();
    restoreSnapshot();
  }, [confirmDiscardIfNeeded, restoreSnapshot]);

  useLayoutEffect(() => {
    if (searchModeUi) {
      return;
    }
    const top = pendingListScrollRestore.current;
    if (top == null) {
      return;
    }
    pendingListScrollRestore.current = null;
    const el = listScrollRef.current;
    if (el) {
      el.scrollTop = top;
    }
  }, [searchModeUi, dirPath, displayRows.length]);

  useLayoutEffect(() => {
    if (searchModeUi || focusPane !== "tree") {
      return;
    }
    scrollListSelectionIntoView(listScrollRef.current, selectedIndex);
  }, [focusPane, searchModeUi, selectedIndex]);

  useLayoutEffect(() => {
    if (!searchModeUi || focusPane !== "tree") {
      return;
    }
    scrollListSelectionIntoView(searchListRef.current, searchSelectedIndex);
  }, [focusPane, searchModeUi, searchSelectedIndex]);

  useEffect(() => {
    const t = firedQuery.trim();
    const p = prevFiredTrim.current;
    if (p !== "" && t === "" && searchSessionRef.current) {
      searchSessionRef.current = false;
      setQueryInput("");
      setSessionSearchScope(null);
      setFocusPane("tree");
      setMobilePanel("entries");
      searchInputRef.current?.blur();
      restoreSnapshot();
    }
    prevFiredTrim.current = t;
  }, [firedQuery, restoreSnapshot]);

  useEffect(() => {
    setSearchSelectedIndex((i) =>
      Math.min(Math.max(0, searchRows.length - 1), i),
    );
  }, [searchRows.length]);

  const onSortColumn = useCallback((column: SortColumn) => {
    setEntrySort((prev) => {
      if (prev?.column === column) {
        return {
          column,
          direction: prev.direction === "asc" ? "desc" : "asc",
        };
      }
      return { column, direction: "asc" };
    });
  }, []);

  const isMobile = useSyncExternalStore(
    (onStoreChange) => {
      const mq = window.matchMedia("(max-width: 768px)");
      mq.addEventListener("change", onStoreChange);
      return () => mq.removeEventListener("change", onStoreChange);
    },
    () => window.matchMedia("(max-width: 768px)").matches,
    () => false,
  );

  useEffect(() => {
    if (
      pendingSearchRestore.current != null ||
      pendingFocusName.current != null
    ) {
      return;
    }
    setSelectedIndex(dirPath === "/" ? 0 : 1);
  }, [dirPath]);

  useLayoutEffect(() => {
    setLoadingList(true);
    setEntries([]);
  }, [dirPath]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setListError(null);
      try {
        const rows = await listDirectory(dirPath);
        if (!cancelled) {
          setEntries(rows);
          setAppReady();
          const focusName = pendingFocusName.current;
          if (focusName != null) {
            pendingFocusName.current = null;
            const ordered = orderEntriesForDisplay(rows, entrySortRef.current);
            const found = ordered.findIndex((e) => e.name === focusName);
            if (found >= 0) {
              const offset = dirPath === "/" ? 1 : 2;
              setSelectedIndex(offset + found);
            }
          } else if (pendingSearchRestore.current != null) {
            const pr = pendingSearchRestore.current;
            pendingSearchRestore.current = null;
            setSelectedIndex(pr.selectedIndex);
            pendingListScrollRestore.current = pr.scrollTop;
          }
        }
      } catch (e) {
        if (!cancelled) {
          pendingFocusName.current = null;
          pendingSearchRestore.current = null;
          setListError(e instanceof Error ? e.message : String(e));
          setEntries([]);
          setAppReady();
        }
      } finally {
        if (!cancelled) {
          setLoadingList(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [dirPath, setAppReady]);

  useEffect(() => {
    if (!openDocPath) {
      setPreviewContent(null);
      setPreviewError(null);
      setPreviewLoading(false);
      return;
    }
    let cancelled = false;
    setPreviewLoading(true);
    setPreviewError(null);
    void (async () => {
      try {
        const doc = await getDocument(openDocPath);
        if (!cancelled) {
          setPreviewContent(doc.content);
        }
      } catch (e) {
        if (!cancelled) {
          setPreviewError(e instanceof Error ? e.message : String(e));
          setPreviewContent(null);
        }
      } finally {
        if (!cancelled) {
          setPreviewLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [openDocPath, docReloadNonce]);

  useEffect(() => {
    setDocumentSurface("preview");
  }, [openDocPath]);

  useLayoutEffect(() => {
    const hq = previewHighlightQuery?.trim() ?? "";
    if (hq.length < 2 || previewContent == null || openDocPath == null) {
      return;
    }
    const el = previewBodyRef.current;
    if (!el) {
      return;
    }
    let id2: number | null = null;
    const id1 = requestAnimationFrame(() => {
      id2 = requestAnimationFrame(() => {
        el
          .querySelector(`mark.${PREVIEW_HIGHLIGHT_CLASS}`)
          ?.scrollIntoView({ block: "center", inline: "nearest" });
      });
    });
    return () => {
      cancelAnimationFrame(id1);
      if (id2 != null) {
        cancelAnimationFrame(id2);
      }
    };
  }, [openDocPath, previewContent, previewHighlightQuery]);

  useEffect(() => {
    return () => {
      if (focusSyncFrame.current != null) {
        cancelAnimationFrame(focusSyncFrame.current);
        focusSyncFrame.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (focusPane !== "tree") {
      return;
    }
    if (focusSyncFrame.current != null) {
      cancelAnimationFrame(focusSyncFrame.current);
    }
    focusSyncFrame.current = requestAnimationFrame(() => {
      focusSyncFrame.current = null;
      if (searchModeUi) {
        if (document.activeElement !== searchInputRef.current) {
          searchListRef.current?.focus();
        }
        return;
      }
      listScrollRef.current?.focus();
    });
    return () => {
      if (focusSyncFrame.current != null) {
        cancelAnimationFrame(focusSyncFrame.current);
        focusSyncFrame.current = null;
      }
    };
  }, [dirPath, focusPane, searchModeUi]);

  // When search is fired, move DOM focus to the results list so right-arrow / enter
  // open the selected hit without caret-position ambiguity on the search input.
  useEffect(() => {
    if (firedQuery.trim() !== "") {
      requestAnimationFrame(() => {
        searchListRef.current?.focus();
      });
    }
  }, [firedQuery]);

  const clearOpen = useCallback(() => {
    if (!confirmDiscardIfNeeded()) {
      return;
    }
    setPreviewHighlightQuery(null);
    setSearchParams(
      (prev) => {
        const n = new URLSearchParams(prev);
        n.delete(OPEN_Q);
        return n;
      },
      { replace: true },
    );
  }, [confirmDiscardIfNeeded, setSearchParams]);

  const focusSearchResults = useCallback(() => {
    setFocusPane("tree");
    requestAnimationFrame(() => {
      searchListRef.current?.focus();
    });
  }, []);

  const goDir = useCallback(
    (path: string) => {
      if (!confirmDiscardIfNeeded()) {
        return;
      }
      setPreviewHighlightQuery(null);
      pendingFocusName.current = null;
      navigate(entriesPathForDir(path));
    },
    [confirmDiscardIfNeeded, navigate],
  );

  const goDirUp = useCallback(() => {
    if (!confirmDiscardIfNeeded()) {
      return;
    }
    setPreviewHighlightQuery(null);
    const seg =
      dirPath.replace(/\/+$/, "").split("/").filter(Boolean).pop() ?? null;
    pendingFocusName.current = seg;
    navigate(entriesPathForDir(parentPath(dirPath)));
  }, [confirmDiscardIfNeeded, dirPath, navigate]);

  const activateRow = useCallback(
    (row: DisplayRow) => {
      if (row.kind === "nav") {
        if (row.nav === "parent") {
          goDirUp();
        } else {
          goDir(row.targetPath);
        }
        setMobilePanel("entries");
        return;
      }
      const e = row.entry;
      const p = childPath(dirPath, e.name);
      if (e.kind === KIND_DIR) {
        goDir(p);
        setMobilePanel("entries");
      } else if (e.kind === KIND_FILE) {
        if (p !== openDocPath && !confirmDiscardIfNeeded()) {
          return;
        }
        setPreviewHighlightQuery(null);
        pendingFocusName.current = null;
        navigate(withOpenDocQuery(dirPath, p));
        setFocusPane("preview");
        requestAnimationFrame(() => {
          previewBodyRef.current?.focus();
        });
        if (isMobile) {
          setMobilePanel("preview");
        }
      }
    },
    [
      confirmDiscardIfNeeded,
      dirPath,
      goDir,
      goDirUp,
      isMobile,
      navigate,
      openDocPath,
    ],
  );

  const previewSearchHit = useCallback(
    (hit: SearchHit) => {
      if (hit.path !== openDocPath && !confirmDiscardIfNeeded()) {
        return;
      }
      const hq = firedQueryRef.current.trim();
      const origin = snapshotRef.current?.dirPath ?? dirPath;
      pendingFocusName.current = null;
      navigate(withOpenDocQuery(origin, hit.path));
      setPreviewHighlightQuery(hq);
      setFocusPane("preview");
      if (isMobile) {
        setMobilePanel("preview");
      }
      requestAnimationFrame(() => {
        previewBodyRef.current?.focus();
      });
    },
    [confirmDiscardIfNeeded, dirPath, isMobile, navigate, openDocPath],
  );

  const activateSearchRow = useCallback(
    (row: SearchListRow) => {
      if (row.kind === "nav") {
        if (row.nav === "parent") {
          exitSearchSession();
        } else {
          searchSessionRef.current = false;
          setFiredQuery("");
          setQueryInput("");
          setSessionSearchScope(null);
          setFocusPane("tree");
          setMobilePanel("entries");
          searchInputRef.current?.blur();
          goDir("/");
        }
        return;
      }
      previewSearchHit(row.hit);
    },
    [exitSearchSession, goDir, previewSearchHit],
  );

  const openSelectedSearchHit = useCallback(() => {
    const row = searchRowsRef.current[searchSelectedIndexRef.current];
    if (!row || row.kind !== "hit") {
      return;
    }
    previewSearchHit(row.hit);
  }, [previewSearchHit]);

  const scrollPreviewBy = useCallback((delta: number) => {
    const el = previewBodyRef.current;
    if (el) {
      el.scrollBy({ top: delta, behavior: "auto" });
    }
  }, []);

  const scrollPreviewPage = useCallback((dir: 1 | -1) => {
    const el = previewBodyRef.current;
    if (el) {
      const step = Math.round(el.clientHeight * 0.85);
      el.scrollBy({ top: dir * step, behavior: "auto" });
    }
  }, []);

  const scrollListPage = useCallback(
    (
      listEl: HTMLUListElement | null,
      currentIndex: number,
      dir: 1 | -1,
      setIdx: (n: number) => void,
    ) => {
      if (!listEl) {
        return;
      }
      const nextIndex = pageMoveIndex(listEl, currentIndex, dir);
      setIdx(nextIndex);
    },
    [],
  );

  const dispatchEntriesKey = useCallback(
    (key: string) => {
      switch (key) {
        case "ArrowUp":
          setSelectedIndex((i) => Math.max(0, i - 1));
          break;
        case "ArrowDown": {
          const rows = displayRowsRef.current;
          setSelectedIndex((i) =>
            Math.min(Math.max(0, rows.length - 1), i + 1),
          );
          break;
        }
        case "PageUp":
          scrollListPage(
            listScrollRef.current,
            selectedIndexRef.current,
            -1,
            setSelectedIndex,
          );
          break;
        case "PageDown":
          scrollListPage(
            listScrollRef.current,
            selectedIndexRef.current,
            1,
            setSelectedIndex,
          );
          break;
        case "Home": {
          const rows = displayRowsRef.current;
          const idx = firstDataEntryIndex(rows);
          setSelectedIndex(idx);
          const ul = listScrollRef.current;
          if (ul) {
            ul.scrollTop = 0;
          }
          break;
        }
        case "End": {
          const rows = displayRowsRef.current;
          if (rows.length > 0) {
            setSelectedIndex(rows.length - 1);
            const ul = listScrollRef.current;
            if (ul) {
              ul.scrollTop = ul.scrollHeight - ul.clientHeight;
            }
          }
          break;
        }
        default:
          break;
      }
    },
    [scrollListPage],
  );

  /** Shared with `useLibraryKeyboard` and search `<input>` when list/preview should move (Ferrum F). */
  const dispatchLibraryKey = useCallback(
    (key: string) => {
      const scrollPreviewVp = libraryKeysScrollPreviewViewport(
        focusPane,
        previewBodyRef.current,
        previewContent,
        previewLoading,
      );
      switch (key) {
        case "ArrowUp":
          if (scrollPreviewVp) {
            scrollPreviewBy(-PREVIEW_SCROLL_STEP);
          } else {
            dispatchEntriesKey(key);
          }
          break;
        case "ArrowDown":
          if (scrollPreviewVp) {
            scrollPreviewBy(PREVIEW_SCROLL_STEP);
          } else {
            dispatchEntriesKey(key);
          }
          break;
        case "PageUp":
          if (scrollPreviewVp) {
            scrollPreviewPage(-1);
          } else {
            dispatchEntriesKey(key);
          }
          break;
        case "PageDown":
          if (scrollPreviewVp) {
            scrollPreviewPage(1);
          } else {
            dispatchEntriesKey(key);
          }
          break;
        case "Home":
          if (scrollPreviewVp) {
            const pel = previewBodyRef.current;
            if (pel) {
              pel.scrollTop = 0;
            }
          } else {
            dispatchEntriesKey(key);
          }
          break;
        case "End":
          if (scrollPreviewVp) {
            const pel = previewBodyRef.current;
            if (pel) {
              pel.scrollTop = pel.scrollHeight - pel.clientHeight;
            }
          } else {
            dispatchEntriesKey(key);
          }
          break;
        default:
          break;
      }
    },
    [
      dispatchEntriesKey,
      focusPane,
      previewContent,
      previewLoading,
      scrollPreviewBy,
      scrollPreviewPage,
    ],
  );

  const fireSearchFromControl = useCallback(() => {
    const trimmed = queryInput.trim();
    const curFired = firedQueryRef.current.trim();
    if (curFired === "") {
      if (trimmed !== "") {
        snapshotRef.current = {
          dirPath,
          selectedIndex,
          scrollTop: listScrollRef.current?.scrollTop ?? 0,
          scope: searchScope,
        };
        setSessionSearchScope(searchScope);
        searchSessionRef.current = true;
        setFiredQuery(trimmed);
        setSearchSelectedIndex(0);
      } else if (focusPane !== "preview") {
        const row = displayRowsRef.current[selectedIndexRef.current];
        if (row) {
          activateRow(row);
        }
      }
      return;
    }
    if (trimmed !== curFired) {
      setFiredQuery(trimmed);
      setSearchSelectedIndex(0);
      return;
    }
    const row = searchRowsRef.current[searchSelectedIndexRef.current];
    if (row?.kind === "hit") {
      previewSearchHit(row.hit);
    } else if (row) {
      activateSearchRow(row);
    }
  }, [
    activateRow,
    activateSearchRow,
    dirPath,
    focusPane,
    previewSearchHit,
    queryInput,
    searchScope,
    selectedIndex,
  ]);

  const onSearchListKeyDown = useCallback(
    (e: KeyboardEvent<HTMLUListElement>) => {
      const consume = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      switch (e.key) {
        case "ArrowUp":
          consume();
          setSearchSelectedIndex((i) => Math.max(0, i - 1));
          break;
        case "ArrowDown": {
          consume();
          const n = searchRowsRef.current.length;
          setSearchSelectedIndex((i) => Math.min(Math.max(0, n - 1), i + 1));
          break;
        }
        case "Home":
          consume();
          setSearchSelectedIndex(0);
          break;
        case "End": {
          consume();
          const n = searchRowsRef.current.length;
          setSearchSelectedIndex(n > 0 ? n - 1 : 0);
          break;
        }
        case "PageUp":
          consume();
          scrollListPage(
            searchListRef.current,
            searchSelectedIndexRef.current,
            -1,
            setSearchSelectedIndex,
          );
          break;
        case "PageDown":
          consume();
          scrollListPage(
            searchListRef.current,
            searchSelectedIndexRef.current,
            1,
            setSearchSelectedIndex,
          );
          break;
        case "Enter":
        case "ArrowRight": {
          consume();
          const row = searchRowsRef.current[searchSelectedIndexRef.current];
          if (row) {
            activateSearchRow(row);
          }
          break;
        }
        case "ArrowLeft":
          consume();
          exitSearchSession();
          break;
        case "Escape":
          consume();
          exitSearchSession();
          break;
        default:
          break;
      }
    },
    [activateSearchRow, exitSearchSession, scrollListPage],
  );

  const kbdEnabled =
    location.pathname.startsWith("/entries") &&
    (!searchModeUi || focusPane === "preview") &&
    documentSurface !== "chat";

  useLibraryKeyboard(kbdEnabled, {
    onArrowUp: () => {
      dispatchLibraryKey("ArrowUp");
    },
    onArrowDown: () => {
      dispatchLibraryKey("ArrowDown");
    },
    onEnter: () => {
      if (focusPane === "preview") {
        return;
      }
      fireSearchFromControl();
    },
    onEscape: () => {
      if (searchModeUi) {
        if (focusPane === "preview") {
          clearOpen();
          focusSearchResults();
          return;
        }
        exitSearchSession();
        return;
      }
      if (isMobile && mobilePanel === "preview") {
        setMobilePanel("entries");
        setFocusPane("tree");
        return;
      }
      clearOpen();
      setFocusPane("tree");
    },
    onArrowLeft: () => {
      if (focusPane === "preview") {
        if (searchModeUi) {
          focusSearchResults();
        } else {
          setFocusPane("tree");
        }
        return;
      }
      if (dirPath !== "/") {
        goDirUp();
        return;
      }
      if (loadingList) {
        return;
      }
      const rows = displayRowsRef.current;
      const firstEntryIdx = rows.findIndex((r) => r.kind === "entry");
      if (firstEntryIdx >= 0) {
        setSelectedIndex(firstEntryIdx);
      }
    },
    onArrowRight: () => {
      if (focusPane === "preview") {
        if (searchModeUi) {
          focusSearchResults();
        }
        return;
      }
      const row = displayRowsRef.current[selectedIndexRef.current];
      if (row) {
        activateRow(row);
      }
    },
    onPageUp: () => {
      dispatchLibraryKey("PageUp");
    },
    onPageDown: () => {
      dispatchLibraryKey("PageDown");
    },
    onHome: () => {
      dispatchLibraryKey("Home");
    },
    onEnd: () => {
      dispatchLibraryKey("End");
    },
  });

  const onSearchKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const el = e.currentTarget;
    if (e.key === "Escape") {
      e.preventDefault();
      if (el.value.trim() !== "") {
        setQueryInput("");
        return;
      }
      if (firedQueryRef.current.trim() !== "") {
        exitSearchSession();
        return;
      }
      searchSessionRef.current = false;
      if (isMobile && mobilePanel === "preview") {
        setMobilePanel("entries");
        setFocusPane("tree");
        return;
      }
      clearOpen();
      setFocusPane("tree");
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      fireSearchFromControl();
      return;
    }
    if (!searchModeUi) {
      if (ENTRIES_DELEGATE_KEYS.has(e.key)) {
        e.preventDefault();
        dispatchEntriesKey(e.key);
      }
      return;
    }
    if (e.key === "ArrowLeft") {
      if (el.selectionStart === 0 && el.selectionEnd === 0) {
        e.preventDefault();
        exitSearchSession();
      }
      return;
    }
    if (e.key === "ArrowRight") {
      if (
        el.selectionStart === el.value.length &&
        el.selectionEnd === el.value.length
      ) {
        e.preventDefault();
        openSelectedSearchHit();
      }
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setSearchSelectedIndex((i) => Math.max(0, i - 1));
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const n = searchRowsRef.current.length;
      setSearchSelectedIndex((i) => Math.min(Math.max(0, n - 1), i + 1));
      return;
    }
    if (e.key === "Home") {
      e.preventDefault();
      setSearchSelectedIndex(0);
      return;
    }
    if (e.key === "End") {
      e.preventDefault();
      const n = searchRowsRef.current.length;
      setSearchSelectedIndex(n > 0 ? n - 1 : 0);
      return;
    }
    if (e.key === "PageUp") {
      e.preventDefault();
      scrollListPage(
        searchListRef.current,
        searchSelectedIndexRef.current,
        -1,
        setSearchSelectedIndex,
      );
      return;
    }
    if (e.key === "PageDown") {
      e.preventDefault();
      scrollListPage(
        searchListRef.current,
        searchSelectedIndexRef.current,
        1,
        setSearchSelectedIndex,
      );
      return;
    }
  };

  const showTree = !isMobile || mobilePanel === "entries";
  const showPreview = !isMobile || mobilePanel === "preview";

  return (
    <>
      {listError ? <div className={styles.errBanner}>{listError}</div> : null}
      <div className={styles.layout}>
        <div className={styles.searchBar}>
          <select
            className={styles.searchScope}
            aria-label="Search scope"
            value={searchModeUi ? (sessionSearchScope ?? "local") : searchScope}
            onChange={(ev) => {
              const v = ev.target.value;
              const newScope = v === "global" ? "global" : "local";
              setSearchScope(newScope);
              if (searchModeUi) {
                if (snapshotRef.current) {
                  snapshotRef.current = {
                    ...snapshotRef.current,
                    scope: newScope,
                  };
                }
                setSessionSearchScope(newScope);
              }
            }}
          >
            <option value="local">Local</option>
            <option value="global">Global</option>
          </select>
          <input
            ref={searchInputRef}
            type="text"
            className={styles.searchInput}
            placeholder="Query the librarium…"
            autoComplete="off"
            aria-label="Search query"
            value={queryInput}
            data-testid="search-input"
            onChange={(ev) => {
              setQueryInput(ev.target.value);
            }}
            onKeyDown={onSearchKeyDown}
          />
          <button
            type="button"
            className={styles.searchBtn}
            data-testid="search-submit"
            disabled={queryInput.trim() === ""}
            onClick={() => {
              fireSearchFromControl();
            }}
          >
            Search
          </button>
        </div>
        <div className={styles.split}>
          <div
            className={`${styles.treePane} ${!showTree ? styles.mobileHidden : ""}`}
          >
            <div className={styles.treePaneStack}>
              {searchModeUi ? (
                <SearchResultsList
                  ref={searchListRef}
                  resultsHeading={`Search results for: ${firedQuery.trim()}`}
                  rows={searchRows}
                  selectedIndex={searchSelectedIndex}
                  focusTree={focusPane === "tree"}
                  loading={searchLoading}
                  error={searchError}
                  onSelectIndex={setSearchSelectedIndex}
                  onActivate={activateSearchRow}
                  onKeyDown={onSearchListKeyDown}
                />
              ) : (
                <FileTree
                  ref={listScrollRef}
                  dirPath={dirPath}
                  rows={treeRows}
                  listEmpty={!loadingList && entries.length === 0}
                  selectedIndex={selectedIndex}
                  focusPane={focusPane}
                  sortState={entrySort}
                  onSortColumn={onSortColumn}
                  onSelectIndex={setSelectedIndex}
                  onActivateRow={activateRow}
                />
              )}
              {loadingList && !searchModeUi ? (
                <div
                  className={styles.treeLoadingOverlay}
                  data-testid="entries-loading"
                >
                  Loading entries…
                </div>
              ) : null}
            </div>
          </div>
          <div
            className={`${styles.previewPaneWrap} ${!showPreview ? styles.mobileHidden : ""}`}
          >
            {isMobile && mobilePanel === "preview" ? (
              <button
                type="button"
                className={`${styles.backBtn} ${styles.mobileOnly}`}
                data-testid="mobile-back"
                onClick={() => {
                  setMobilePanel("entries");
                  setFocusPane("tree");
                }}
              >
                ← Back
              </button>
            ) : null}
            <PreviewPane
              key={openDocPath ?? "__no_open_doc__"}
              ref={previewBodyRef}
              pathLabel={openDocPath}
              content={previewContent}
              loading={previewLoading}
              error={previewError}
              highlightQuery={previewHighlightQuery}
              documentSurface={documentSurface}
              onDocumentSurfaceChange={setDocumentSurface}
              onEditSessionChange={onEditSessionChange}
              onDocumentSaved={bumpOpenDocumentReload}
              onPreviewContentSynced={setPreviewContent}
              onRequestDocumentReload={bumpOpenDocumentReload}
            />
          </div>
        </div>
      </div>
    </>
  );
}
