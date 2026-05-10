import { lazy, Suspense, useEffect, useState } from "react";
import type { ChartData } from "chart.js";
import { listDirectory, rpcCall } from "../../api/client";
import { useAppShell } from "../../app/appShellContext";
import { KIND_DIR, KIND_FILE } from "../entries/entryModel";
import styles from "./StatsView.module.scss";

const StatsChart = lazy(() => import("./StatsChart"));

function childPath(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}

interface Totals {
  files: number;
  dirs: number;
  bytes: number;
  chartSlices: { label: string; count: number }[];
}

/** Full-tree walk for aggregate cards (no extension breakdown). */
async function walkTotals(): Promise<Pick<Totals, "files" | "dirs" | "bytes">> {
  let files = 0;
  let dirs = 0;
  let bytes = 0;
  const stack: string[] = ["/"];

  while (stack.length > 0) {
    const p = stack.pop()!;
    const rows = await listDirectory(p);
    for (const e of rows) {
      const cp = childPath(p, e.name);
      if (e.kind === KIND_FILE) {
        files += 1;
        bytes += e.size_bytes ?? 0;
      } else if (e.kind === KIND_DIR) {
        dirs += 1;
        stack.push(cp);
      }
    }
  }

  return { files, dirs, bytes };
}

/** Count every file under `startDir` (recursive). */
async function countFilesUnder(startDir: string): Promise<number> {
  let n = 0;
  const stack = [startDir];
  while (stack.length > 0) {
    const p = stack.pop()!;
    const rows = await listDirectory(p);
    for (const e of rows) {
      if (e.kind === KIND_FILE) {
        n += 1;
      } else {
        stack.push(childPath(p, e.name));
      }
    }
  }
  return n;
}

/** Top-level folder doc counts + `(root)` for loose root files (Logis). */
async function collectTopLevelChartSlices(): Promise<
  { label: string; count: number }[]
> {
  const root = await listDirectory("/");
  const slices: { label: string; count: number }[] = [];

  let rootFiles = 0;
  for (const e of root) {
    if (e.kind === KIND_FILE) {
      rootFiles += 1;
    }
  }
  if (rootFiles > 0) {
    slices.push({ label: "(root)", count: rootFiles });
  }

  const topDirs = root
    .filter((e) => e.kind === KIND_DIR)
    .sort((a, b) => a.name.localeCompare(b.name));

  for (const d of topDirs) {
    const path = childPath("/", d.name);
    const count = await countFilesUnder(path);
    slices.push({ label: d.name, count });
  }

  return slices;
}

const mbFmt2 = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

interface WhoamiPermissions {
  allow?: { read?: string[]; write?: string[] };
  deny?: { read?: string[]; write?: string[] };
}

interface Whoami {
  acl: string | null;
  admin: boolean;
  permissions: WhoamiPermissions | unknown;
}

const mbFmt1 = new Intl.NumberFormat(undefined, {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

const intFmt = new Intl.NumberFormat();

function formatMegabytes(bytes: number): string {
  if (bytes <= 0) {
    return `${mbFmt2.format(0)} MB`;
  }
  const mb = bytes / (1024 * 1024);
  if (mb < 0.01) {
    return "<0.01 MB";
  }
  if (mb < 10) {
    return `${mbFmt2.format(mb)} MB`;
  }
  return `${mbFmt1.format(mb)} MB`;
}

const CHART_COLORS = [
  "#c9a227",
  "#4a6fa5",
  "#6b8f71",
  "#8b5a6b",
  "#5a8f8f",
  "#a67c52",
  "#6b6b8f",
  "#8ec8e8",
];

function chartFromSlices(
  slices: { label: string; count: number }[],
): ChartData<"pie"> {
  const labels = slices.map((s) => s.label);
  const data = slices.map((s) => s.count);
  const backgroundColor = labels.map(
    (_, i) => CHART_COLORS[i % CHART_COLORS.length],
  );
  return {
    labels,
    datasets: [
      {
        data,
        backgroundColor,
        borderColor: "#1a1f26",
        borderWidth: 1,
      },
    ],
  };
}

function normalizePaths(raw: string[] | undefined): string[] {
  if (!raw || !Array.isArray(raw)) return [];
  return raw.filter((s) => typeof s === "string" && s.trim() !== "");
}

function PathColumn({
  title,
  paths,
}: {
  title: string;
  paths: string[];
}) {
  return (
    <div className={styles.aclPathColumn}>
      <div className={styles.aclPathColumnTitle}>{title}</div>
      {paths.length === 0 ? (
        <span className={styles.aclPathEmpty}>—</span>
      ) : (
        <ul className={styles.aclPathList}>
          {paths.map((p, i) => (
            <li key={`${title}:${i}:${p}`} className={styles.aclPathItem}>
              {p}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function AclIdentityPanel({ who }: { who: Whoami }) {
  const perms =
    typeof who.permissions === "object" && who.permissions !== null
      ? (who.permissions as WhoamiPermissions)
      : {};
  const allowRead = normalizePaths(perms.allow?.read);
  const allowWrite = normalizePaths(perms.allow?.write);
  const denyRead = normalizePaths(perms.deny?.read);
  const denyWrite = normalizePaths(perms.deny?.write);

  return (
    <section className={styles.aclSeal} data-testid="stats-acl-panel">
      <div className={styles.aclSealTop}>
        <span className={styles.aclSealMark} aria-hidden>
          ◈
        </span>
        <div className={styles.aclSealHeading}>
          <span className={styles.aclSealKicker}>Lexicanum seal</span>
          <h2 className={styles.aclSealName}>{who.acl?.trim() || "—"}</h2>
        </div>
      </div>
      <div className={styles.aclSealBadgeRow}>
        <span
          className={who.admin ? styles.aclBadgeAdmin : styles.aclBadgeScoped}
          data-testid="stats-acl-role"
        >
          {who.admin ? "Administrator" : "Scoped operator"}
        </span>
      </div>
      {who.admin ? (
        <p className={styles.aclAdminLitany}>
          Writ across the whole librarium — deny/allow lists waived by the Golden Throne.
        </p>
      ) : (
        <div className={styles.aclMatrix}>
          <div className={styles.aclMatrixRow}>
            <PathColumn title="Allow · read" paths={allowRead} />
            <PathColumn title="Allow · write" paths={allowWrite} />
          </div>
          <div className={styles.aclMatrixRow}>
            <PathColumn title="Deny · read" paths={denyRead} />
            <PathColumn title="Deny · write" paths={denyWrite} />
          </div>
        </div>
      )}
    </section>
  );
}

export default function StatsView() {
  const { setAppReady } = useAppShell();
  const [err, setErr] = useState<string | null>(null);
  const [totals, setTotals] = useState<Totals | null>(null);
  const [whoami, setWhoami] = useState<Whoami | null>(null);
  const [statsLimited, setStatsLimited] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const who = await rpcCall<Whoami>("whoami", {});
        if (!cancelled && !who.admin) {
          setWhoami(who);
          setStatsLimited(true);
          setAppReady();
          return;
        }
        const [agg, chartSlices] = await Promise.all([
          walkTotals(),
          collectTopLevelChartSlices(),
        ]);
        if (!cancelled) {
          setTotals({ ...agg, chartSlices });
          setWhoami(who);
          setAppReady();
        }
      } catch (e) {
        if (!cancelled) {
          setErr(e instanceof Error ? e.message : String(e));
          setAppReady();
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [setAppReady]);

  if (err) {
    return (
      <div className={styles.root} data-testid="stats-root">
        <p className={styles.err}>{err}</p>
      </div>
    );
  }

  if (!totals && !statsLimited) {
    return (
      <div className={styles.root} data-testid="stats-root">
        <p>Scanning the librarium…</p>
      </div>
    );
  }

  if (statsLimited && whoami) {
    return (
      <div className={styles.root} data-testid="stats-root">
        <AclIdentityPanel who={whoami} />
        <p className={styles.chartEmpty}>
          Aggregate totals and charts are hidden for non-administrators.
        </p>
      </div>
    );
  }

  if (!totals) {
    return null;
  }

  const chartData =
    totals.chartSlices.length > 0 ? chartFromSlices(totals.chartSlices) : null;

  return (
    <div className={styles.root} data-testid="stats-root">
      {whoami ? <AclIdentityPanel who={whoami} /> : null}
      <div className={styles.grid}>
        <div className={styles.card} data-testid="stats-total-files">
          <div className={styles.cardLabel}>Files</div>
          <div className={styles.cardValue}>{intFmt.format(totals.files)}</div>
        </div>
        <div className={styles.card} data-testid="stats-total-dirs">
          <div className={styles.cardLabel}>Directories</div>
          <div className={styles.cardValue}>{intFmt.format(totals.dirs)}</div>
        </div>
        <div className={styles.card} data-testid="stats-total-bytes">
          <div className={styles.cardLabel}>Total size</div>
          <div className={styles.cardValue} data-testid="stats-total-mb">
            {formatMegabytes(totals.bytes)}
          </div>
        </div>
      </div>
      <div className={styles.chartWrap} data-testid="stats-chart">
        {chartData ? (
          <Suspense fallback={<p>Loading chart…</p>}>
            <StatsChart data={chartData} />
          </Suspense>
        ) : (
          <p className={styles.chartEmpty}>No files to chart yet.</p>
        )}
      </div>
    </div>
  );
}
