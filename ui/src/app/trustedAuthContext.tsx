import { createContext, useContext } from "react";

/** Present only under `AuthShell` while the forgegate has cleared (`phase === "ok"`). */
export interface TabulariumTrustedAuthValue {
  /** True when the server requires API keys — otherwise no PSK chrome. */
  authenticateApi: boolean;
  /** Clears cookie + session PSK and returns to the gate — the Emperor revokes this seal. */
  logoutPskSession: () => void;
}

export const TabulariumTrustedAuthContext =
  createContext<TabulariumTrustedAuthValue | null>(null);

export function useTabulariumTrustedAuth(): TabulariumTrustedAuthValue | null {
  return useContext(TabulariumTrustedAuthContext);
}
