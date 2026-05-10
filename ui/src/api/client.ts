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

interface JsonRpcError {
  code?: number;
  message?: string;
  data?: unknown;
}

interface JsonRpcResponse<T> {
  jsonrpc?: string;
  result?: T;
  error?: JsonRpcError;
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

export async function rpcCall<T>(
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  const r = await request("/rpc", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      method,
      params,
      id: Date.now(),
    }),
  });
  if (!r.ok) {
    throw new Error(`rpc ${method} failed: ${r.status}`);
  }
  const j = (await r.json()) as JsonRpcResponse<T>;
  if (j.error) {
    const msg = j.error.message?.trim();
    throw new Error(msg && msg !== "" ? msg : `rpc ${method} failed`);
  }
  if (!("result" in j)) {
    throw new Error(`rpc ${method} invalid response`);
  }
  return j.result as T;
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

export async function createDirectory(
  path: string,
  description: string | null = null,
): Promise<void> {
  const r = await request("/api/doc", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      path,
      description,
      parents: false,
    }),
  });
  if (!r.ok) {
    throw new Error(`create_directory failed: ${r.status}`);
  }
}

export async function createDocument(
  path: string,
  content = "",
): Promise<void> {
  await rpcCall<number>("create_document", { path, content });
}

export async function getDocument(apiPath: string): Promise<DocumentBody> {
  const enc = encodeApiPath(apiPath);
  const r = await request(`/api/doc${enc}`);
  if (!r.ok) {
    throw new Error(`get_document failed: ${r.status}`);
  }
  return (await r.json()) as DocumentBody;
}

export async function renameDocument(
  path: string,
  newName: string,
): Promise<void> {
  await rpcCall<null>("rename_document", { path, new_name: newName });
}

export async function renameDirectory(
  path: string,
  newPath: string,
): Promise<void> {
  await rpcCall<null>("rename_directory", { path, new_path: newPath });
}

export async function setEntryDescription(
  path: string,
  description: string,
): Promise<void> {
  await rpcCall<null>("describe", { path, description });
}

export async function deleteEntry(
  path: string,
  recursive: boolean,
): Promise<void> {
  const enc = encodeApiPath(path);
  const q = recursive ? "?recursive=true" : "";
  const r = await request(`/api/doc${enc}${q}`, {
    method: "DELETE",
  });
  if (!r.ok) {
    throw new Error(`delete failed: ${r.status}`);
  }
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
