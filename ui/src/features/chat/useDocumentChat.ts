import { useCallback, useEffect, useRef, useState } from "react";

import { documentWsUrl } from "./documentWsUrl";

export type DocumentChatStatus =
  | "idle"
  | "connecting"
  | "open"
  | "error"
  | "closed";

/**
 * Live document tail over `GET /ws`. Emits **`say` only** from the client — never `append`
 * (Ferrum / meetings/webui.chat).
 *
 * Transcript is the **full subscribed document body** (or tail per `lines`); for v1 the UI
 * renders it as markdown — mixed chat/non-chat in one file is a known constraint.
 */
export function useDocumentChat(
  path: string | null,
  enabled: boolean,
): {
  transcript: string;
  status: DocumentChatStatus;
  errorMessage: string | null;
  reconnect: () => void;
  sendSay: (text: string, fromId: string) => boolean;
} {
  const [transcript, setTranscript] = useState("");
  const [status, setStatus] = useState<DocumentChatStatus>("idle");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [reconnectTick, setReconnectTick] = useState(0);

  const wsRef = useRef<WebSocket | null>(null);
  const genRef = useRef(0);
  const pathRef = useRef<string | null>(path);
  pathRef.current = path;

  const reconnect = useCallback(() => {
    setReconnectTick((n) => n + 1);
  }, []);

  const sendSay = useCallback((text: string, fromId: string) => {
    const ws = wsRef.current;
    const p = pathRef.current;
    const nick = fromId.trim();
    if (!ws || ws.readyState !== WebSocket.OPEN || !p || nick === "") {
      return false;
    }
    ws.send(
      JSON.stringify({
        op: "say",
        path: p,
        from_id: nick,
        data: text,
      }),
    );
    return true;
  }, []);

  useEffect(() => {
    if (!enabled || !path) {
      genRef.current += 1;
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
      setTranscript("");
      setStatus("idle");
      setErrorMessage(null);
      return;
    }

    const gen = ++genRef.current;
    setStatus("connecting");
    setErrorMessage(null);

    const ws = new WebSocket(documentWsUrl());
    wsRef.current = ws;

    ws.onopen = () => {
      if (gen !== genRef.current) {
        ws.close();
        return;
      }
      ws.send(
        JSON.stringify({
          op: "subscribe",
          path,
          lines: "+1",
        }),
      );
    };

    ws.onmessage = (ev) => {
      if (gen !== genRef.current) {
        return;
      }
      let msg: { op: string; data?: string; message?: string };
      try {
        msg = JSON.parse(ev.data as string) as {
          op: string;
          data?: string;
          message?: string;
        };
      } catch {
        setStatus("error");
        setErrorMessage("invalid server message");
        return;
      }
      if (msg.op === "error") {
        setStatus("error");
        setErrorMessage(msg.message ?? "websocket error");
        return;
      }
      if (msg.op === "reset" && typeof msg.data === "string") {
        setTranscript(msg.data);
        setStatus("open");
        return;
      }
      if (msg.op === "append" && typeof msg.data === "string") {
        setTranscript((prev) => prev + msg.data!);
        setStatus("open");
      }
    };

    ws.onerror = () => {
      if (gen !== genRef.current) {
        return;
      }
      setStatus("error");
      setErrorMessage("connection failed");
    };

    ws.onclose = () => {
      if (gen !== genRef.current) {
        return;
      }
      wsRef.current = null;
      setStatus((prev) => (prev === "open" ? "closed" : prev));
    };

    return () => {
      genRef.current += 1;
      ws.close();
      wsRef.current = null;
    };
  }, [enabled, path, reconnectTick]);

  return {
    transcript,
    status,
    errorMessage,
    reconnect,
    sendSay,
  };
}
