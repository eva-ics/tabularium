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

/** Guest `/api/whoami` — optional `anonymous` tells OIDC assertion apart from `[anonymous]`. */
async function fetchWhoamiBody(): Promise<{ anonymous?: boolean } | null> {
  try {
    const w = await fetch("/api/whoami");
    if (!w.ok || w.status !== 200) {
      return null;
    }
    return (await w.json()) as { anonymous?: boolean };
  } catch {
    return null;
  }
}

async function whoamiOkWithoutKey(): Promise<boolean> {
  return (await fetchWhoamiBody()) !== null;
}

export function AuthShell() {
  const [phase, setPhase] = useState<"load" | "gate" | "ok">("load");
  const [authenticateApi, setAuthenticateApi] = useState(true);
  const [oidcAssertionAuth, setOidcAssertionAuth] = useState(false);
  const [anonymousAuth, setAnonymousAuth] = useState(false);
  const [anonymousAvailable, setAnonymousAvailable] = useState(false);
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
        const j = (await r.json()) as {
          authenticate_api?: boolean;
          oidc_enabled?: boolean;
        };
        if (cancelled) return;
        if (!j.authenticate_api) {
          setAuthenticateApi(false);
          setOidcAssertionAuth(false);
          setAnonymousAuth(false);
          setAnonymousAvailable(false);
          setPhase("ok");
          return;
        }
        setAuthenticateApi(true);
        if (j.oidc_enabled === true) {
          // Guest `[anonymous]` whoami also succeeds — only a real assertion skips the PSK flow.
          const who = await fetchWhoamiBody();
          if (cancelled) return;
          if (who !== null && who.anonymous !== true) {
            setOidcAssertionAuth(true);
            setAnonymousAuth(false);
            setAnonymousAvailable(false);
            clearStoredPsk();
            setAuthHeaderProvider(() => null);
            setPhase("ok");
            return;
          }
        }
        setOidcAssertionAuth(false);

        const anonOk = await whoamiOkWithoutKey();
        if (cancelled) return;
        setAnonymousAvailable(anonOk);

        const fromSession = sessionStorage.getItem(PSK_SESSION_STORAGE_KEY);
        if (fromSession && fromSession.trim() !== "") {
          const k = fromSession.trim();
          const v = await verifyPskWithServer(k);
          if (cancelled) return;
          if (v.ok) {
            setAnonymousAuth(false);
            setAuthHeaderProvider(() => ({ "X-Auth-Key": k }));
            setPhase("ok");
          } else {
            clearStoredPsk();
            setAuthHeaderProvider(() => null);
            if (anonOk) {
              setAnonymousAuth(true);
              setPhase("ok");
            } else {
              setPhase("gate");
            }
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
            setAnonymousAuth(false);
            setAuthHeaderProvider(() => ({ "X-Auth-Key": k }));
            setPhase("ok");
          } else {
            clearStoredPsk();
            setAuthHeaderProvider(() => null);
            if (anonOk) {
              setAnonymousAuth(true);
              setPhase("ok");
            } else {
              setPhase("gate");
            }
          }
          return;
        }
        setAuthHeaderProvider(() => null);
        if (anonOk) {
          setAnonymousAuth(true);
          setPhase("ok");
        } else {
          setPhase("gate");
        }
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
    void (async () => {
      const anonOk = await whoamiOkWithoutKey();
      setAnonymousAvailable(anonOk);
      if (anonOk) {
        setAnonymousAuth(true);
        setPhase("ok");
      } else {
        setAnonymousAuth(false);
        setPhase("gate");
      }
    })();
  }, [clearStoredPsk]);

  const startPskLogin = useCallback(() => {
    setGateError(null);
    setPhase("gate");
  }, []);

  const trustedCtx = useMemo<TabulariumTrustedAuthValue>(
    () => ({
      authenticateApi,
      oidcAssertionAuth,
      anonymousAuth,
      logoutPskSession,
      startPskLogin,
    }),
    [
      authenticateApi,
      oidcAssertionAuth,
      anonymousAuth,
      logoutPskSession,
      startPskLogin,
    ],
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
                  const el = inputRef.current;
                  if (el) {
                    el.value = "";
                    window.requestAnimationFrame(() => el.focus());
                  }
                  return;
                }
                const trust = fd.get("trust") === "on";
                sessionStorage.setItem(PSK_SESSION_STORAGE_KEY, psk);
                if (trust) writeTrustedPskCookie(psk);
                else clearTrustedPskCookie();
                setAnonymousAuth(false);
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
              <p
                id="tabularium-psk-error"
                className={styles.gateErr}
                role="alert"
              >
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
            <button
              type="submit"
              className={styles.enterBtn}
              disabled={gateBusy}
            >
              {gateBusy ? "…" : "Enter"}
            </button>
            {anonymousAvailable ? (
              <button
                type="button"
                className={styles.enterBtn}
                disabled={gateBusy}
                data-testid="psk-continue-anonymous"
                onClick={() => {
                  setGateError(null);
                  setAnonymousAuth(true);
                  setAuthHeaderProvider(() => null);
                  setPhase("ok");
                }}
              >
                Continue without key
              </button>
            ) : null}
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
