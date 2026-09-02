import { lazy, Suspense } from "react";
import {
  BrowserRouter,
  Navigate,
  Route,
  Routes,
  useLocation,
} from "react-router-dom";
import { AppShell } from "./app/AppShell";
import { AuthShell } from "./app/AuthShell";
import { entriesUrlFromLibrariumPathname } from "./features/entries/entriesPath";
import { EntriesView } from "./features/entries/EntriesView";

const StatsView = lazy(() => import("./features/stats/StatsView"));

function LibrariumPathRedirect() {
  const { pathname, hash } = useLocation();
  return (
    <Navigate
      to={`${entriesUrlFromLibrariumPathname(pathname)}${hash}`}
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
