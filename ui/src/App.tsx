import { lazy, Suspense, useEffect, useState } from "react";
import {
  BrowserRouter,
  Navigate,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";
import { listDirectory } from "./api/client";
import { AppShell } from "./app/AppShell";
import { AuthShell } from "./app/AuthShell";
import { KIND_FILE } from "./features/entries/entryModel";
import {
  entriesUrlFromLibrariumPathname,
  librariumTargetFromPathname,
} from "./features/entries/entriesPath";
import { EntriesView } from "./features/entries/EntriesView";

const StatsView = lazy(() => import("./features/stats/StatsView"));

/** Resolves the entry kind from its parent listing; unknown targets open as a directory. */
async function isLibrariumDocument(pathname: string): Promise<boolean> {
  const target = librariumTargetFromPathname(pathname);
  if (target === null) {
    return false;
  }
  try {
    const entries = await listDirectory(target.parent);
    const entry = entries.find((e) => e.name === target.name);
    return entry?.kind === KIND_FILE;
  } catch {
    return false;
  }
}

function LibrariumPathRedirect() {
  const { pathname, hash } = useLocation();
  const [resolved, setResolved] = useState<{
    pathname: string;
    isDocument: boolean;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void isLibrariumDocument(pathname).then((isDocument) => {
      if (!cancelled) {
        setResolved({ pathname, isDocument });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pathname]);

  if (resolved === null || resolved.pathname !== pathname) {
    return <p>Opening…</p>;
  }
  return (
    <Navigate
      to={`${entriesUrlFromLibrariumPathname(pathname, resolved.isDocument)}${hash}`}
      replace
    />
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AuthShell />}>
          <Route element={<AppShell />}>
            <Route path="/" element={<Navigate to="/entries" replace />} />
            <Route path="/entries/*" element={<EntriesView />} />
            <Route
              path="/stats"
              element={
                <Suspense fallback={<p>Loading stats…</p>}>
                  <StatsView />
                </Suspense>
              }
            />
            <Route path="*" element={<LibrariumPathRedirect />} />
          </Route>
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
