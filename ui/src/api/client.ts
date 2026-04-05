/** All HTTP to the librarium API goes through here — no raw `fetch` in components. */

export type AuthHeaderFn = () => Record<string, string> | null;

let getAuthHeaders: AuthHeaderFn = () => null;

export function setAuthHeaderProvider(fn: AuthHeaderFn): void {
  getAuthHeaders = fn;
}

export interface ListedEntry {
  id: number;
  kind: number;
  name: string;
  description: string | null;
  created_at: number | null;
  modified_at: number | null;
  accessed_at: number | null;
  size_bytes: number | null;
  recursive_file_count: number | null;
}

export interface DocumentBody {
  id: number;
  path: string;
  content: string;
  created_at: number | null;
  modified_at: number | null;
  accessed_at: number | null;
  size_bytes: number | null;
}

async function request(path: string, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers);
  const auth = getAuthHeaders();
  if (auth) {
    for (const [k, v] of Object.entries(auth)) {
      headers.set(k, v);
    }
  }
  return fetch(path, { ...init, headers });
}

export async function listDirectory(apiPath: string): Promise<ListedEntry[]> {
  const enc = encodeApiPath(apiPath);
  const url = enc === "/" ? "/api/doc" : `/api/doc${enc}`;
  const r = await request(url);
  if (!r.ok) {
    throw new Error(`list_directory failed: ${r.status}`);
  }
  const j = (await r.json()) as unknown;
  if (!Array.isArray(j)) {
    throw new Error("list_directory: expected array");
  }
  return j as ListedEntry[];
}

export async function getDocument(apiPath: string): Promise<DocumentBody> {
  const enc = encodeApiPath(apiPath);
  const r = await request(`/api/doc${enc}`);
  if (!r.ok) {
    throw new Error(`get_document failed: ${r.status}`);
  }
  return (await r.json()) as DocumentBody;
}

/** Replace document body (REST PUT); same path rules as {@link getDocument}. */
export async function putDocument(
  apiPath: string,
  content: string,
): Promise<void> {
  const enc = encodeApiPath(apiPath);
  const r = await request(`/api/doc${enc}`, {
    method: "PUT",
    headers: { "Content-Type": "text/plain; charset=utf-8" },
    body: content,
  });
  if (!r.ok) {
    let detail = "";
    try {
      detail = await r.text();
    } catch {
      /* ignore */
    }
    const msg = detail.trim() || `put_document failed: ${r.status}`;
    throw new Error(msg);
  }
}

export interface SearchHit {
  document_id: number;
  path: string;
  snippet: string;
  score: number;
  line_number: number | null;
}

export interface SearchDocumentsOptions {
  /** Absolute directory path; limits search to that subtree. Omit for global. */
  subtreeDir?: string | null;
}

export async function searchDocuments(
  query: string,
  options?: SearchDocumentsOptions,
): Promise<SearchHit[]> {
  const q = query.trim();
  if (q === "") {
    return [];
  }
  const params = new URLSearchParams();
  params.set("q", q);
  const sub = options?.subtreeDir?.trim();
  if (sub != null && sub !== "") {
    params.set("dir", sub);
  }
  const url = `/api/search?${params.toString()}`;
  const r = await request(url);
  if (!r.ok) {
    throw new Error(`search failed: ${r.status}`);
  }
  const j = (await r.json()) as unknown;
  if (!Array.isArray(j)) {
    throw new Error("search: expected array");
  }
  return j as SearchHit[];
}

/** Encode path for URL segments after /api/doc — leading slash, segments encoded. */
function encodeApiPath(path: string): string {
  const t = path.trim();
  if (t === "" || t === "/") {
    return "/";
  }
  const noLead = t.replace(/^\/+/, "");
  const parts = noLead.split("/").filter(Boolean);
  const enc = parts.map((p) => encodeURIComponent(p)).join("/");
  return `/${enc}`;
}
