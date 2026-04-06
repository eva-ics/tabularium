import {
  forwardRef,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type KeyboardEvent,
} from "react";
import ReactMarkdown from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { defaultSchema } from "hast-util-sanitize";
import type { Schema } from "hast-util-sanitize";
import { getDocument, putDocument } from "../../api/client";
import { DocumentChatBody } from "../chat/DocumentChatBody";
import {
  extractSearchHighlightTerms,
  PREVIEW_HIGHLIGHT_CLASS,
  rehypeSearchHighlight,
  splitPlainTextForPreview,
} from "./highlightSearchTerms";
import {
  extendSchemaWithGfmTables,
  gfmTableSanitizeSchema,
} from "../../markdown/gfmSanitize";
import styles from "./PreviewPane.module.scss";

const sanitizeWithMark: Schema = extendSchemaWithGfmTables({
  ...gfmTableSanitizeSchema,
  tagNames: [...(gfmTableSanitizeSchema.tagNames ?? []), "mark"],
  attributes: {
    ...defaultSchema.attributes,
    mark: ["className", "class"],
  },
});

export interface EditSessionState {
  active: boolean;
  dirty: boolean;
}

export type DocumentSurface = "preview" | "chat";

interface PreviewPaneProps {
  pathLabel: string | null;
  content: string | null;
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
    const [rawMode, setRawMode] = useState(false);
    const [editMode, setEditMode] = useState(false);
    const [draft, setDraft] = useState("");
    const [baseline, setBaseline] = useState("");
    const [saveError, setSaveError] = useState<string | null>(null);
    const [saving, setSaving] = useState(false);
    const [editLoading, setEditLoading] = useState(false);
    const [editPrepareError, setEditPrepareError] = useState<string | null>(
      null,
    );

    const highlightTrimmed = highlightQuery?.trim() ?? "";
    const highlightActive = highlightTrimmed.length >= 2;

    const rehypePlugins = useMemo(() => {
      const sanitize: [typeof rehypeSanitize, Schema] = [
        rehypeSanitize,
        highlightActive ? sanitizeWithMark : gfmTableSanitizeSchema,
      ];
      return highlightActive
        ? [rehypeSearchHighlight(highlightTrimmed), sanitize]
        : [sanitize];
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
      documentSurface === "preview" && !editMode && showDoc;
    const showChatBody =
      documentSurface === "chat" && !editMode && pathLabel != null;

    return (
      <section
        className={styles.pane}
        data-testid="preview-pane"
        aria-label="Document preview"
      >
        <div className={styles.header}>
          <span className={styles.headerTitle}>
            {pathLabel ? pathLabel : "Preview"}
          </span>
          {showRawToggle || showChatToggle || editMode ? (
            <div className={styles.headerActions}>
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
                    setRawMode((r) => !r);
                  }}
                >
                  {rawMode ? "MD" : "RAW"}
                </button>
              ) : null}
              {showChatToggle ? (
                <button
                  type="button"
                  className={styles.chatToggle}
                  data-testid="preview-chat"
                  disabled={editLoading}
                  onClick={toggleChatSurface}
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
                  disabled={editLoading}
                  onClick={() => {
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
          {error ? <p className={styles.err}>{error}</p> : null}
          {editPrepareError && !editMode ? (
            <p className={styles.saveErr} data-testid="preview-edit-load-error">
              {editPrepareError}
            </p>
          ) : null}
          {loading ? <p className={styles.empty}>Loading…</p> : null}
          {!loading && !error && content === null ? (
            <p className={styles.empty}>Select a file — the Emperor watches.</p>
          ) : null}
          {editMode && pathLabel !== null ? (
            <div className={styles.editArea}>
              {saveError ? (
                <p className={styles.saveErr} data-testid="preview-save-error">
                  {saveError}
                </p>
              ) : null}
              <textarea
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
          {showPreviewBody && !rawMode ? (
            <div className={styles.markdown}>
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                rehypePlugins={rehypePlugins}
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
