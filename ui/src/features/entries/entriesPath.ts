/**
 * Browser routes for the entries browser — `/entries` + optional path segments.
 * Optional `?open=` holds absolute document path for preview + refresh (Enginseer / Ferrum).
 * Human links may omit `/entries` (`/projects/…/doc.md`); those map via
 * `entriesUrlFromLibrariumPathname`.
 */

const PREFIX = "/entries";

function decodePathSegments(pathname: string): string[] {
  return pathname
    .split("/")
    .filter(Boolean)
    .map((s) => {
      try {
        return decodeURIComponent(s);
      } catch {
        return s;
      }
    });
}

function absFromPathname(pathname: string): string {
  const segments = decodePathSegments(pathname);
  return segments.length === 0 ? "/" : `/${segments.join("/")}`;
}

/** Document directory path from location (always a directory in the URL). */
export function dirPathFromEntriesLocation(pathname: string): string {
  if (pathname === PREFIX || pathname === `${PREFIX}/`) {
    return "/";
  }
  if (!pathname.startsWith(`${PREFIX}/`)) {
    return "/";
  }
  const rest = pathname.slice(PREFIX.length + 1);
  const segments = decodePathSegments(rest);
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

/** Absolute path, parent directory, and last segment of a human librarium URL. */
export interface LibrariumTarget {
  abs: string;
  parent: string;
  name: string;
}

/** Splits a human librarium URL (`/a/b` → `/a`, `b`); `null` for the root. */
export function librariumTargetFromPathname(
  pathname: string,
): LibrariumTarget | null {
  const abs = absFromPathname(pathname);
  if (abs === "/") {
    return null;
  }
  const slash = abs.lastIndexOf("/");
  return {
    abs,
    parent: slash <= 0 ? "/" : abs.slice(0, slash),
    name: abs.slice(slash + 1),
  };
}

/**
 * Maps a human librarium URL (`/projects/…/doc.md`) to the entries browser route.
 * `isDocument` must come from the server entry kind, not from the segment name.
 */
export function entriesUrlFromLibrariumPathname(
  pathname: string,
  isDocument: boolean,
): string {
  const target = librariumTargetFromPathname(pathname);
  if (target === null) {
    return PREFIX;
  }
  if (isDocument) {
    return withOpenDocQuery(target.parent, target.abs);
  }
  return entriesPathForDir(target.abs);
}
