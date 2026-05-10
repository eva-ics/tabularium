import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Outlet } from "react-router-dom";
import { setAuthHeaderProvider } from "../api/client";
import styles from "./AuthShell.module.scss";
import {
  PSK_SESSION_STORAGE_KEY,
  clearTrustedPskCookie,
  readTrustedPskCookie,
  writeTrustedPskCookie,
} from "./pskCookie";
import {
  TabulariumTrustedAuthContext,
  type TabulariumTrustedAuthValue,
} from "./trustedAuthContext";
import { verifyPskWithServer } from "./verifyPsk";

export function AuthShell() {
  const [phase, setPhase] = useState<"load" | "gate" | "ok">("load");
  const [authenticateApi, setAuthenticateApi] = useState(true);
  const [gateError, setGateError] = useState<string | null>(null);
  const [gateBusy, setGateBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const clearStoredPsk = useCallback(() => {
    clearTrustedPskCookie();
    sessionStorage.removeItem(PSK_SESSION_STORAGE_KEY);
  }, []);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const r = await fetch("/api/test");
        const j = (await r.json()) as { authenticate_api?: boolean };
        if (cancelled) return;
        if (!j.authenticate_api) {
          setAuthenticateApi(false);
          setPhase("ok");
          return;
        }
        setAuthenticateApi(true);
        const fromSession = sessionStorage.getItem(PSK_SESSION_STORAGE_KEY);
        if (fromSession && fromSession.trim() !== "") {
          const k = fromSession.trim();
          const v = await verifyPskWithServer(k);
          if (cancelled) return;
          if (v.ok) {
            setAuthHeaderProvider(() => ({ "X-Auth-Key": k }));
            setPhase("ok");
          } else {
            clearStoredPsk();
            setAuthHeaderProvider(() => null);
            setPhase("gate");
          }
          return;
        }
        const fromCookie = readTrustedPskCookie();
        if (fromCookie && fromCookie.trim() !== "") {
          const k = fromCookie.trim();
          const v = await verifyPskWithServer(k);
          if (cancelled) return;
          if (v.ok) {
            sessionStorage.setItem(PSK_SESSION_STORAGE_KEY, k);
            setAuthHeaderProvider(() => ({ "X-Auth-Key": k }));
            setPhase("ok");
          } else {
            clearStoredPsk();
            setAuthHeaderProvider(() => null);
            setPhase("gate");
          }
          return;
        }
        setPhase("gate");
      } catch {
        if (!cancelled) setPhase("gate");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [clearStoredPsk]);

  useEffect(() => {
    if (phase !== "gate") return;
    const id = window.requestAnimationFrame(() => {
      inputRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(id);
  }, [phase]);

  const logoutPskSession = useCallback(() => {
    clearStoredPsk();
    setAuthHeaderProvider(() => null);
    setGateError(null);
    setPhase("gate");
  }, [clearStoredPsk]);

  const trustedCtx = useMemo<TabulariumTrustedAuthValue>(
    () => ({
      authenticateApi,
      logoutPskSession,
    }),
    [authenticateApi, logoutPskSession],
  );

  if (phase === "load") {
    return <div className={styles.loadRoot} aria-busy="true" />;
  }

  if (phase === "gate") {
    return (
      <div className={styles.gateRoot}>
        <div className={styles.card}>
          <form
            className={styles.form}
            onSubmit={(e) => {
              e.preventDefault();
              void (async () => {
                const fd = new FormData(e.currentTarget);
                const psk = String(fd.get("psk") ?? "").trim();
                if (!psk) return;
                setGateBusy(true);
                setGateError(null);
                const v = await verifyPskWithServer(psk);
                setGateBusy(false);
                if (!v.ok) {
                  setGateError(v.message);
                  return;
                }
                const trust = fd.get("trust") === "on";
                sessionStorage.setItem(PSK_SESSION_STORAGE_KEY, psk);
                if (trust) writeTrustedPskCookie(psk);
                else clearTrustedPskCookie();
                setAuthHeaderProvider(() => ({ "X-Auth-Key": psk }));
                setPhase("ok");
              })();
            }}
          >
            <label className={styles.label} htmlFor="tabularium-psk">
              PSK
            </label>
            <input
              ref={inputRef}
              id="tabularium-psk"
              className={styles.input}
              name="psk"
              type="password"
              autoComplete="off"
              autoFocus
              disabled={gateBusy}
              aria-invalid={gateError ? true : undefined}
              aria-describedby={gateError ? "tabularium-psk-error" : undefined}
            />
            {gateError ? (
              <p id="tabularium-psk-error" className={styles.gateErr} role="alert">
                {gateError}
              </p>
            ) : null}
            <label className={styles.trustRow}>
              <input
                name="trust"
                type="checkbox"
                className={styles.trustCb}
                disabled={gateBusy}
              />
              <span className={styles.trustLabel}>Trust this computer</span>
            </label>
            <button type="submit" className={styles.enterBtn} disabled={gateBusy}>
              {gateBusy ? "…" : "Enter"}
            </button>
          </form>
        </div>
      </div>
    );
  }

  return (
    <TabulariumTrustedAuthContext.Provider value={trustedCtx}>
      <Outlet />
    </TabulariumTrustedAuthContext.Provider>
  );
}
