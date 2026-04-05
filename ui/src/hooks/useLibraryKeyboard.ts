import { useEffect, useRef } from "react";

export interface LibraryKeyboardHandlers {
  onArrowUp: () => void;
  onArrowDown: () => void;
  onEnter: () => void;
  onEscape: () => void;
  onArrowLeft: () => void;
  onArrowRight: () => void;
  /** Preview pane: page scroll (optional). */
  onPageUp?: () => void;
  onPageDown?: () => void;
  onHome?: () => void;
  onEnd?: () => void;
}

/**
 * Single keyboard rite for the library UI — no scattered `onKeyDown` in panes.
 */
export function useLibraryKeyboard(
  enabled: boolean,
  handlers: LibraryKeyboardHandlers,
): void {
  const ref = useRef(handlers);
  const enabledRef = useRef(enabled);
  /** Render-phase sync so keydown never sees a stale handler (Left-at-root rite). */
  ref.current = handlers;
  enabledRef.current = enabled;

  useEffect(() => {
    if (!enabled) {
      return;
    }
    const onKey = (e: KeyboardEvent) => {
      if (!enabledRef.current || e.defaultPrevented) {
        return;
      }
      const t = e.target;
      if (
        t instanceof HTMLInputElement ||
        t instanceof HTMLTextAreaElement ||
        (t instanceof HTMLElement && t.isContentEditable)
      ) {
        return;
      }
      const h = ref.current;
      switch (e.key) {
        case "ArrowUp":
          e.preventDefault();
          h.onArrowUp();
          break;
        case "ArrowDown":
          e.preventDefault();
          h.onArrowDown();
          break;
        case "Enter":
          e.preventDefault();
          h.onEnter();
          break;
        case "Escape":
          e.preventDefault();
          h.onEscape();
          break;
        case "ArrowLeft":
          e.preventDefault();
          h.onArrowLeft();
          break;
        case "ArrowRight":
          e.preventDefault();
          h.onArrowRight();
          break;
        case "PageUp":
          if (h.onPageUp) {
            e.preventDefault();
            h.onPageUp();
          }
          break;
        case "PageDown":
          if (h.onPageDown) {
            e.preventDefault();
            h.onPageDown();
          }
          break;
        case "Home":
          if (h.onHome) {
            e.preventDefault();
            h.onHome();
          }
          break;
        case "End":
          if (h.onEnd) {
            e.preventDefault();
            h.onEnd();
          }
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled]);
}
