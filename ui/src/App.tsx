import { lazy, Suspense } from "react";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "./app/AppShell";
import { EntriesView } from "./features/entries/EntriesView";

const StatsView = lazy(() => import("./features/stats/StatsView"));

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
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
          <Route path="*" element={<Navigate to="/entries" replace />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
