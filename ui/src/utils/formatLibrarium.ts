const intFmt = new Intl.NumberFormat();

/** `size_bytes` from API — grouped digits + ` B`. */
export function formatBytes(n: number | null): string {
  if (n == null) {
    return "";
  }
  return `${intFmt.format(n)} B`;
}

/** `recursive_file_count` — grouped. */
export function formatCount(n: number | null): string {
  if (n == null) {
    return "";
  }
  return intFmt.format(n);
}

/** REST sends `bma_ts::Timestamp` as nanoseconds (u64). */
export function formatModifiedAt(ts: number | null): string {
  if (ts == null || ts === 0) {
    return "";
  }
  const ms = ts > 1e15 ? ts / 1e6 : ts;
  return new Date(ms).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
