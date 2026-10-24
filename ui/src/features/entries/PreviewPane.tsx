import {
  forwardRef,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import ReactMarkdown from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import type { Schema } from "hast-util-sanitize";
import { useNavigate } from "react-router-dom";
import { getDocument, putDocument, rpcCall } from "../../api/client";
import { DocumentChatBody } from "../chat/DocumentChatBody";
import {
  extractSearchHighlightTerms,
  PREVIEW_HIGHLIGHT_CLASS,
  rehypeSearchHighlight,
  splitPlainTextForPreview,
} from "./highlightSearchTerms";
import { gfmTableKatexSanitizeSchema } from "../../markdown/gfmSanitize";
import {
  markdownRemarkPlugins,
  rehypeKatex,
} from "../../markdown/mathMarkdown";
import { copyTextClipboard } from "../../utils/clipboard";
import { parentPath } from "./entryModel";
import { withOpenDocQuery } from "./entriesPath";
import styles from "./PreviewPane.module.scss";
import { codePreviewNodes, documentPreviewFormat } from "./documentPreview";

const sanitizeWithMark: Schema = {
  ...gfmTableKatexSanitizeSchema,
  tagNames: [...(gfmTableKatexSanitizeSchema.tagNames ?? []), "mark"],
  attributes: {
    ...gfmTableKatexSanitizeSchema.attributes,
    mark: ["className", "class"],
  },
};

export interface EditSessionState {
  active: boolean;
  dirty: boolean;
}

export type DocumentSurface = "preview" | "chat";

interface PreviewPaneProps {
  pathLabel: string | null;
  content: string | null;
  mimeType?: string | null;
  loading: boolean;
  error: string | null;
  /** When set (e.g. after opening a search hit), matching terms are highlighted. */
  highlightQuery: string | null;
  documentSurface: DocumentSurface;
  onDocumentSurfaceChange: (surface: DocumentSurface) => void;
  fullscreen: boolean;
  onToggleFullscreen: () => void;
  /** Fires when edit mode or dirty flag changes (navigation guard in parent). */
  onEditSessionChange?: (state: EditSessionState) => void;
  /** After successful PUT; parent should refetch document body. */
  onDocumentSaved?: () => void;
  /** After opening the editor with a freshly fetched body (avoids stale preview vs chat/remote). */
  onPreviewContentSynced?: (body: string) => void;
  /** Bump parent reload (e.g. `docReloadNonce`) on preview/RAW/MD switches — fresh server body (meetings/webui.chat). */
  onRequestDocumentReload?: () => void;
}

export const PreviewPane = forwardRef<HTMLDivElement, PreviewPaneProps>(
  function PreviewPane(
    {
      pathLabel,
      content,
      mimeType,
      loading,
      error,
      highlightQuery,
      documentSurface,
      onDocumentSurfaceChange,
      fullscreen,
      onToggleFullscreen,
      onEditSessionChange,
      onDocumentSaved,
      onPreviewContentSynced,
      onRequestDocumentReload,
    },
    ref,
  ) {
    const navigate = useNavigate();
    const editorRef = useRef<HTMLTextAreaElement>(null);
    const [rawMode, setRawMode] = useState(false);
    const [metaMode, setMetaMode] = useState(false);
    const [metadata, setMetadata] = useState<string | null>(null);
    const [metadataError, setMetadataError] = useState<string | null>(null);
    useEffect(() => { setMetaMode(false); }, [pathLabel]);
    useEffect(() => {
      let active = true;
      setMetadata(null);
      setMetadataError(null);
      if (metaMode && pathLabel) {
        void rpcCall<{ metadata: Record<string, unknown> }>("get_document_metadata", {path: pathLabel})
          .then((value) => { if (active) setMetadata(JSON.stringify(value.metadata, null, 2)); })
          .catch((e: unknown) => { if (active) setMetadataError(String(e)); });
      }
      return () => { active = false; };
    }, [metaMode, pathLabel]);
    const [editMode, setEditMode] = useState(false);
    const [draft, setDraft] = useState("");
    const [baseline, setBaseline] = useState("");
    const [saveError, setSaveError] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);
    const [editLoading, setEditLoading] = useState(false);
    const [copied, setCopied] = useState(false);
    const [copyFailed, setCopyFailed] = useState(false);
    useEffect(() => { setCopyFailed(false); setCopied(false); }, [pathLabel]);
    const [editPrepareError, setEditPrepareError] = useState<string | null>(
      null,
    );

    const format = documentPreviewFormat(pathLabel, mimeType);
    const highlightTrimmed = highlightQuery?.trim() ?? "";
    const highlightActive = highlightTrimmed.length >= 2;

    const rehypePlugins = useMemo(() => {
      const schema = highlightActive
        ? sanitizeWithMark
        : gfmTableKatexSanitizeSchema;
      const sanitize: [typeof rehypeSanitize, Schema] = [
        rehypeSanitize,
        schema,
      ];
      return highlightActive
        ? [rehypeSearchHighlight(highlightTrimmed), rehypeKatex, sanitize]
        : [rehypeKatex, sanitize];
    }, [highlightActive, highlightTrimmed]);

    const rawTerms = useMemo(
      () => extractSearchHighlightTerms(highlightTrimmed),
      [highlightTrimmed],
    );

    const rawSegments = useMemo(() => {
      if (!content || !highlightActive || rawTerms.length === 0) {
        return null;
      }
      return splitPlainTextForPreview(content, rawTerms);
    }, [content, highlightActive, rawTerms]);

    const codeNodes = useMemo(() => {
      if (format === "markdown" || content == null || rawMode) return null;
      return codePreviewNodes(content, format, highlightActive ? rawTerms : []);
    }, [content, format, rawMode, highlightActive, rawTerms]);

    const openDocDir = useMemo(() => {
      if (!pathLabel) {
        return null;
      }
      return parentPath(pathLabel);
    }, [pathLabel]);

    const resolveHref = useCallback(
      (
        hrefRaw: string,
      ): { kind: "passthrough" } | { kind: "navigate"; url: string } => {
        const href = hrefRaw.trim();
        if (href === "") {
          return { kind: "passthrough" };
        }
        if (href.startsWith("#")) {
          return { kind: "passthrough" };
        }
        const lower = href.toLowerCase();
        if (
          lower.startsWith("http://") ||
          lower.startsWith("https://") ||
          lower.startsWith("mailto:") ||
          lower.startsWith("tel:")
        ) {
          return { kind: "passthrough" };
        }

        const [pathPart, hashPart] = href.split("#", 2);
        const hashSuffix =
          hashPart != null && hashPart !== "" ? `#${hashPart}` : "";
        const decodedPath = (() => {
          try {
            return decodeURIComponent(pathPart);
          } catch {
            return pathPart;
          }
        })();

        const normalizeAbs = (absPath: string): string => {
          const segs = absPath.split("/").filter(Boolean);
          const out: string[] = [];
          for (const s of segs) {
            if (s === "." || s === "") {
              continue;
            }
            if (s === "..") {
              if (out.length > 0) {
                out.pop();
              }
              continue;
            }
            out.push(s);
          }
          return `/${out.join("/")}`;
        };

        const baseDir = openDocDir ?? "/";
        const targetAbs = decodedPath.startsWith("/")
          ? normalizeAbs(decodedPath)
          : normalizeAbs(
              baseDir === "/" ? `/${decodedPath}` : `${baseDir}/${decodedPath}`,
            );

        const targetDir = parentPath(targetAbs);
        const url = `${withOpenDocQuery(targetDir, targetAbs)}${hashSuffix}`;
        return { kind: "navigate", url };
      },
      [openDocDir],
    );

    const markdownComponents = useMemo(() => {
      return {
        a: ({
          href,
          children,
          ...props
        }: {
          href?: string;
          children?: ReactNode;
          onClick?: (e: MouseEvent<HTMLAnchorElement>) => void;
        }) => {
          const safeHref = href ?? "";
          const action = resolveHref(safeHref);
          if (action.kind === "navigate") {
            return (
              <a
                href={action.url}
                {...props}
                onClick={(e) => {
                  props.onClick?.(e);
                  if (e.defaultPrevented) {
                    return;
                  }
                  e.preventDefault();
                  navigate(action.url);
                }}
              >
                {children}
              </a>
            );
          }
          return (
            <a href={safeHref} {...props}>
              {children}
            </a>
          );
        },
      };
    }, [navigate, resolveHref]);

    const showDoc = !loading && !error && content !== null;
    const docIdentified = pathLabel != null && !error;
    const showRawToggle =
      docIdentified && documentSurface === "preview" && (showDoc || loading);
    const showFullscreenToggle =
      docIdentified &&
      (documentSurface === "preview" || documentSurface === "chat") &&
      !editMode &&
      (showDoc || loading);
    const showChatToggle = pathLabel != null && showDoc && !editMode;
    const dirty = editMode && draft !== baseline;
    const canSave = dirty && !saving;

    useEffect(() => {
      onEditSessionChange?.({ active: editMode, dirty });
    }, [editMode, dirty, onEditSessionChange]);

    useEffect(() => {
      if (!editMode) {
        return;
      }
      const id = requestAnimationFrame(() => {
        editorRef.current?.focus();
        const len = editorRef.current?.value.length ?? 0;
        editorRef.current?.setSelectionRange(len, len);
      });
      return () => {
        cancelAnimationFrame(id);
      };
    }, [editMode]);

    const handleSave = useCallback(async () => {
      if (!pathLabel || saving || draft === baseline) {
        return;
      }
      setSaving(true);
      setSaveError(null);
      try {
        await putDocument(pathLabel, draft);
        setEditMode(false);
        setSaveError(null);
        onDocumentSaved?.();
      } catch (e) {
        setSaveError(e instanceof Error ? e.message : String(e));
      } finally {
        setSaving(false);
      }
    }, [pathLabel, saving, draft, baseline, onDocumentSaved]);

    const onEditorKeyDown = useCallback(
      (e: KeyboardEvent<HTMLTextAreaElement>) => {
        if ((e.ctrlKey || e.metaKey) && e.key === "s") {
          e.preventDefault();
          if (canSave) {
            void handleSave();
          }
        }
      },
      [canSave, handleSave],
    );

    const startEdit = useCallback(async () => {
      if (pathLabel === null || saving || editLoading) {
        return;
      }
      setEditPrepareError(null);
      setEditLoading(true);
      try {
        const doc = await getDocument(pathLabel);
        setBaseline(doc.content);
        setDraft(doc.content);
        setSaveError(null);
        onPreviewContentSynced?.(doc.content);
        setEditMode(true);
      } catch (e) {
        setEditPrepareError(e instanceof Error ? e.message : String(e));
      } finally {
        setEditLoading(false);
      }
    }, [pathLabel, saving, editLoading, onPreviewContentSynced]);

    const cancelEdit = useCallback(() => {
      setEditMode(false);
      setSaveError(null);
      setEditPrepareError(null);
      setDraft("");
      setBaseline("");
    }, []);

    const toggleChatSurface = useCallback(() => {
      if (documentSurface === "chat") {
        onRequestDocumentReload?.();
        onDocumentSurfaceChange("preview");
      } else {
        onDocumentSurfaceChange("chat");
      }
    }, [documentSurface, onDocumentSurfaceChange, onRequestDocumentReload]);

    const showPreviewBody =
      documentSurface === "preview" && !editMode && !metaMode && showDoc;
    const showChatBody =
      documentSurface === "chat" && !editMode && !metaMode && pathLabel != null;

    return (
      <section
        className={styles.pane}
        data-testid="preview-pane"
        aria-label="Document preview"
      >
        {copyFailed && pathLabel ? (
          <label role="alert">
            Could not copy automatically. Select and copy the path:
            <input aria-label="Document path to copy" value={pathLabel} readOnly
              onFocus={(event) => event.currentTarget.select()} />
          </label>
        ) : null}
        <div className={styles.header}>
          <span className={styles.headerLeft}>
            <span className={styles.headerTitle} data-testid="preview-path" title={pathLabel ?? undefined}>
              {pathLabel ? pathLabel : "Preview"}
            </span>
            {pathLabel ? (
              <button
                type="button"
                className={styles.copyPath}
                title="Copy path"
                onClick={() => {
                  setCopyFailed(false);
                  void copyTextClipboard(pathLabel).then(() => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1500);
                  }).catch(() => setCopyFailed(true));
                }}
              >
                {copied ? "✓" : "⎘"}
              </button>
            ) : null}
          </span>
          {showRawToggle || showChatToggle || editMode ? (
            <div className={styles.headerActions} data-testid="preview-actions">
              {pathLabel && !editMode ? (
                <button type="button" className={styles.rawToggle} data-testid="preview-meta"
                  aria-pressed={metaMode} disabled={loading}
                  onClick={() => setMetaMode((value) => !value)}>{metaMode ? "File" : "Meta"}</button>
              ) : null}
              {showFullscreenToggle ? (
                <button
                  type="button"
                  className={styles.fullToggle}
                  data-testid="preview-full-toggle"
                  disabled={loading}
                  onClick={() => {
                    onToggleFullscreen();
                  }}
                >
                  {fullscreen ? "Dock" : "Full"}
                </button>
              ) : null}
              {showRawToggle ? (
                <button
                  type="button"
                  className={styles.rawToggle}
                  data-testid="preview-raw-toggle"
                  disabled={editMode || loading}
                  onClick={() => {
                    onRequestDocumentReload?.();
                    setMetaMode(false);
                    setRawMode((r) => !r);
                  }}
                >
                  {rawMode ? (format === "markdown" ? "MD" : "PREVIEW") : "RAW"}
                </button>
              ) : null}
              {showChatToggle ? (
                <button
                  type="button"
                  className={styles.chatToggle}
                  data-testid="preview-chat"
                  disabled={editLoading}
                  onClick={() => { setMetaMode(false); toggleChatSurface(); }}
                >
                  {documentSurface === "chat" ? "Preview" : "Chat"}
                </button>
              ) : null}
              {editMode ? (
                <>
                  <button
                    type="button"
                    className={styles.editSave}
                    data-testid="preview-save"
                    disabled={!canSave}
                    onClick={() => {
                      void handleSave();
                    }}
                  >
                    {saving ? "Saving…" : "Save"}
                  </button>
                  <button
                    type="button"
                    className={styles.editCancel}
                    data-testid="preview-cancel-edit"
                    disabled={saving}
                    onClick={cancelEdit}
                  >
                    Cancel
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className={styles.editSave}
                  data-testid="preview-edit"
                  title="Edit document body"
                  disabled={editLoading || metaMode}
                  onClick={() => {
                    setMetaMode(false);
                    void startEdit();
                  }}
                >
                  {editLoading ? "Loading…" : "Edit"}
                </button>
              )}
            </div>
          ) : null}
        </div>
        <div
          ref={showChatBody ? undefined : ref}
          className={`${styles.body} ${editMode ? styles.bodyEditing : ""} ${showChatBody ? styles.bodyChat : ""}`}
          tabIndex={-1}
        >
          {metaMode ? (
            <div data-testid="preview-metadata" aria-label="Read-only document metadata">
              {metadataError ? <p className={styles.err}>{metadataError}</p> :
                metadata === null ? <p>Loading metadata…</p> : <pre className={styles.metadataJson}>{metadata}</pre>}
            </div>
          ) : null}
          {error ? <p className={styles.err}>{error}</p> : null}
          {editPrepareError && !editMode ? (
            <p className={styles.saveErr} data-testid="preview-edit-load-error">
              {editPrepareError}
            </p>
          ) : null}
          {loading ? (
            <p className={styles.empty} data-print-hide>
              Loading…
            </p>
          ) : null}
          {!loading && !error && content === null ? (
            <p className={styles.empty}>Select a file to preview.</p>
          ) : null}
          {editMode && pathLabel !== null ? (
            <div className={styles.editArea}>
              {saveError ? (
                <p className={styles.saveErr} data-testid="preview-save-error">
                  {saveError}
                </p>
              ) : null}
              <textarea
                ref={editorRef}
                className={styles.editor}
                data-testid="preview-editor"
                aria-label="Edit document"
                value={draft}
                onChange={(ev) => {
                  setDraft(ev.target.value);
                }}
                onKeyDown={onEditorKeyDown}
              />
            </div>
          ) : null}
          {showPreviewBody && rawMode ? (
            rawSegments && highlightActive ? (
              <pre className={styles.pre}>
                {rawSegments.map((s, i) =>
                  s.kind === "text" ? (
                    <span key={i}>{s.value}</span>
                  ) : (
                    <mark key={i} className={PREVIEW_HIGHLIGHT_CLASS}>
                      {s.value}
                    </mark>
                  ),
                )}
              </pre>
            ) : (
              <pre className={styles.pre}>{content}</pre>
            )
          ) : null}
          {showPreviewBody && !rawMode && format !== "markdown" ? (
            <pre className={styles.codePreview} data-testid="preview-code" data-language={format}>
              <code>{codeNodes}</code>
            </pre>
          ) : null}
          {showPreviewBody && !rawMode && format === "markdown" ? (
            <div className={`${styles.markdown} markdown`}>
              <ReactMarkdown
                remarkPlugins={markdownRemarkPlugins}
                rehypePlugins={rehypePlugins}
                components={markdownComponents}
              >
                {content}
              </ReactMarkdown>
            </div>
          ) : null}
          {showChatBody ? (
            <DocumentChatBody ref={ref} path={pathLabel!} documentSurfaceChat />
          ) : null}
        </div>
      </section>
    );
  },
);
