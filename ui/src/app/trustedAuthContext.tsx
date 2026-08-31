import { createContext, useContext } from "react";

/** Present only under `AuthShell` while the forgegate has cleared (`phase === "ok"`). */
export interface TabulariumTrustedAuthValue {
  /** True when the server requires API keys — otherwise no PSK chrome. */
  authenticateApi: boolean;
  /** `[oidc]` on server — Web UI skips PSK gate (proxy supplies JWT assertion). */
  oidcAssertionAuth: boolean;
  /** No PSK stored; caller uses configured `[anonymous]` path ACL. */
  anonymousAuth: boolean;
  /** Clears cookie + session PSK and returns to anonymous (or the gate). */
  logoutPskSession: () => void;
  /** Open the PSK gate from an anonymous public session. */
  startPskLogin: () => void;
}

export const TabulariumTrustedAuthContext =
  createContext<TabulariumTrustedAuthValue | null>(null);

export function useTabulariumTrustedAuth(): TabulariumTrustedAuthValue | null {
  return useContext(TabulariumTrustedAuthContext);
}
