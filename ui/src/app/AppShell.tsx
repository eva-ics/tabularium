import { useEffect, useMemo, useState } from "react";
import { NavLink, Outlet } from "react-router-dom";
import { AppShellContext } from "./appShellContext";
import styles from "./AppShell.module.scss";

export function AppShell() {
  const [ready, setReady] = useState(false);
  const ctx = useMemo(
    () => ({
      setAppReady: () => setReady(true),
    }),
    [],
  );

  useEffect(() => {
    document.body.dataset.tabulariumReady = ready ? "true" : "false";
  }, [ready]);

  return (
    <AppShellContext.Provider value={ctx}>
      <div className={styles.shell}>
        <header className={styles.topNav}>
          <span className={styles.brandRow}>
            <img
              className={styles.brandLogo}
              src="/tb.png"
              alt=""
              width={20}
              height={20}
            />
            <span className={styles.brand}>Tabularium</span>
          </span>
          <NavLink
            to="/entries"
            end={false}
            data-testid="top-nav-entries"
            className={({ isActive }) =>
              `${styles.navBtn} ${isActive ? styles.navBtnActive : ""}`
            }
          >
            Entries
          </NavLink>
          <NavLink
            to="/stats"
            data-testid="top-nav-stats"
            className={({ isActive }) =>
              `${styles.navBtn} ${isActive ? styles.navBtnActive : ""}`
            }
          >
            Stats
          </NavLink>
        </header>
        <main className={styles.main}>
          <Outlet />
        </main>
      </div>
    </AppShellContext.Provider>
  );
}
