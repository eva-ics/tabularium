import { createContext, useContext } from "react";

export interface AppShellContextValue {
  setAppReady: () => void;
}

export const AppShellContext = createContext<AppShellContextValue | null>(null);

export function useAppShell(): AppShellContextValue {
  const v = useContext(AppShellContext);
  if (!v) {
    throw new Error("useAppShell outside AppShellContext");
  }
  return v;
}
