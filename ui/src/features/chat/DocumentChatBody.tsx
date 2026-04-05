import {
  forwardRef,
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type ForwardedRef,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";
import ReactMarkdown from "react-markdown";
import rehypeSanitize from "rehype-sanitize";

import {
  readChatNicknameFromCookie,
  writeChatNicknameCookie,
} from "./chatNicknameCookie";
import { type DocumentChatStatus, useDocumentChat } from "./useDocumentChat";
import styles from "../entries/PreviewPane.module.scss";

const NICK_CMD = /^\/nick\s+(.+)$/;

interface DocumentChatBodyProps {
  path: string;
  documentSurfaceChat: boolean;
}

function assignTranscriptRef(
  el: HTMLDivElement | null,
  inner: RefObject<HTMLDivElement | null>,
  outer: ForwardedRef<HTMLDivElement>,
): void {
  inner.current = el;
  if (typeof outer === "function") {
    outer(el);
  } else if (outer) {
    outer.current = el;
  }
}

/**
 * Chat transcript renders stored document markdown; the server may include non-chat prefixes —
 * v1 accepts full body as the scroll (meetings/webui.chat).
 */
export const DocumentChatBody = forwardRef<
  HTMLDivElement,
  DocumentChatBodyProps
>(function DocumentChatBody({ path, documentSurfaceChat }, ref) {
  const [gateNick, setGateNick] = useState("");
  const [sessionNick, setSessionNick] = useState<string | null>(null);
  const [chatLive, setChatLive] = useState(false);
  const [composer, setComposer] = useState("");
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  const composerRef = useRef<HTMLTextAreaElement | null>(null);

  useLayoutEffect(() => {
    if (!documentSurfaceChat) {
      return;
    }
    const c = readChatNicknameFromCookie();
    if (c) {
      setSessionNick(c);
      setChatLive(true);
    } else {
      setSessionNick(null);
      setChatLive(false);
      setGateNick("");
    }
  }, [documentSurfaceChat, path]);

  const hookEnabled = documentSurfaceChat && chatLive && sessionNick != null;
  const { transcript, status, errorMessage, reconnect, sendSay } =
    useDocumentChat(path, hookEnabled);

  useLayoutEffect(() => {
    const el = transcriptRef.current;
    if (!el) {
      return;
    }
    el.scrollTop = el.scrollHeight;
  }, [transcript, hookEnabled]);

  useLayoutEffect(() => {
    if (!documentSurfaceChat || !chatLive) {
      return;
    }
    composerRef.current?.focus();
  }, [documentSurfaceChat, chatLive, path]);

  const trySend = useCallback((): boolean => {
    const nick = sessionNick?.trim() ?? "";
    const raw = composer;
    const trimmed = raw.trim();
    if (nick === "" || trimmed === "") {
      return true;
    }
    const nickMatch = raw.match(NICK_CMD);
    if (nickMatch) {
      const next = nickMatch[1]!.trim();
      if (next !== "") {
        writeChatNicknameCookie(next);
        setSessionNick(next);
        setComposer("");
      }
      return true;
    }
    const ok = sendSay(trimmed, nick);
    if (ok) {
      setComposer("");
    }
    return ok;
  }, [composer, sendSay, sessionNick]);

  const onComposerKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter" || e.shiftKey) {
      return;
    }
    e.preventDefault();
    if (composer.trim() === "") {
      return;
    }
    void trySend();
  };

  const canSend =
    sessionNick != null &&
    sessionNick.trim() !== "" &&
    composer.trim() !== "" &&
    status === "open";

  const onGateSubmit = (ev: FormEvent) => {
    ev.preventDefault();
    const n = gateNick.trim();
    if (n === "") {
      return;
    }
    writeChatNicknameCookie(n);
    setSessionNick(n);
    setChatLive(true);
  };

  if (!documentSurfaceChat) {
    return null;
  }

  if (!chatLive) {
    return (
      <div className={styles.chatGate} data-testid="chat-gate">
        <form className={styles.chatGateForm} onSubmit={onGateSubmit}>
          <label className={styles.chatGateLabel} htmlFor="chat-gate-nick">
            Enter your nickname
          </label>
          <input
            id="chat-gate-nick"
            className={styles.chatGateInput}
            data-testid="chat-gate-input"
            value={gateNick}
            onChange={(e) => {
              setGateNick(e.target.value);
            }}
            autoComplete="off"
            autoFocus
            aria-label="Chat nickname"
          />
          <button
            type="submit"
            className={styles.chatStartBtn}
            data-testid="chat-start"
            disabled={gateNick.trim() === ""}
          >
            Start
          </button>
        </form>
      </div>
    );
  }

  return (
    <div className={styles.chatWrap}>
      {errorMessage != null && status !== "open" ? (
        <div className={styles.chatBanner}>
          <span className={styles.chatBannerText}>{errorMessage}</span>
          <button
            type="button"
            className={styles.chatReconnect}
            data-testid="chat-reconnect"
            onClick={reconnect}
          >
            Retry
          </button>
        </div>
      ) : null}
      {statusBanner(status)}
      <div
        ref={(el) => {
          assignTranscriptRef(el, transcriptRef, ref);
        }}
        className={`${styles.markdown} ${styles.chatTranscript}`}
        data-testid="chat-transcript"
      >
        <ReactMarkdown rehypePlugins={[rehypeSanitize]}>
          {transcript}
        </ReactMarkdown>
      </div>
      <div className={styles.chatComposerRow}>
        <textarea
          ref={composerRef}
          className={styles.chatComposer}
          data-testid="chat-composer"
          aria-label="Chat message"
          rows={4}
          value={composer}
          onChange={(e) => {
            setComposer(e.target.value);
          }}
          onKeyDown={onComposerKeyDown}
        />
        <button
          type="button"
          className={styles.chatSend}
          data-testid="chat-send"
          disabled={!canSend}
          onClick={() => {
            void trySend();
          }}
        >
          Send
        </button>
      </div>
    </div>
  );
});

function statusBanner(status: DocumentChatStatus): ReactNode {
  if (status === "connecting") {
    return (
      <p className={styles.chatStatus} data-testid="chat-status">
        Connecting…
      </p>
    );
  }
  if (status === "closed") {
    return (
      <p className={styles.chatStatus} data-testid="chat-status">
        Disconnected.
      </p>
    );
  }
  return null;
}
