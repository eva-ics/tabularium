/**
 * Browser routes for the entries browser — `/entries` + optional path segments.
 * Optional `?open=` holds absolute document path for preview + refresh (Enginseer / Ferrum).
 */

const PREFIX = "/entries";

/** Librarium dir path from location (always a directory in the URL). */
export function dirPathFromEntriesLocation(pathname: string): string {
  if (pathname === PREFIX || pathname === `${PREFIX}/`) {
    return "/";
  }
  if (!pathname.startsWith(`${PREFIX}/`)) {
    return "/";
  }
  const rest = pathname.slice(PREFIX.length + 1);
  const segments = rest
    .split("/")
    .filter(Boolean)
    .map((s) => {
      try {
        return decodeURIComponent(s);
      } catch {
        return s;
      }
    });
  return `/${segments.join("/")}`;
}

/** React Router pathname for a directory path. */
export function entriesPathForDir(dirPath: string): string {
  if (dirPath === "/" || dirPath === "") {
    return PREFIX;
  }
  const trimmed = dirPath.replace(/^\/+/, "");
  const segments = trimmed
    .split("/")
    .filter(Boolean)
    .map((s) => encodeURIComponent(s));
  return `${PREFIX}/${segments.join("/")}`;
}

export const ENTRIES_PREFIX = PREFIX;

export function withOpenDocQuery(
  dirPath: string,
  docAbsolutePath: string,
): string {
  const path = entriesPathForDir(dirPath);
  const q = new URLSearchParams();
  q.set("open", docAbsolutePath);
  return `${path}?${q.toString()}`;
}
