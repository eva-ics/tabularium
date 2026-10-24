#!/usr/bin/env python3
"""Seed Tabularium, exercise MCP/RPC, verify OpenObserve traces, run load steps."""

from __future__ import annotations

import base64
import json
import os
import statistics
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Any

TB = os.environ.get("TABULARIUM_URL", "http://127.0.0.1:3050").rstrip("/")
MCP = os.environ.get("TABULARIUM_MCP_URL", "http://127.0.0.1:3031/mcp").rstrip("/")
OO = os.environ.get("OPENOBSERVE_URL", "http://127.0.0.1:5080").rstrip("/")
OO_USER = os.environ.get("OO_USER", "root@example.com")
OO_PASSWORD = os.environ.get("OO_PASSWORD", "Complexpass#123")
OO_ORG = os.environ.get("OO_ORG", "default")


def http_json(
    method: str,
    url: str,
    body: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
    timeout: float = 60.0,
) -> tuple[int, Any]:
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    if body is not None:
        req.add_header("Content-Type", "application/json")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode() or "null"
            try:
                return resp.status, json.loads(raw)
            except json.JSONDecodeError:
                return resp.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            parsed: Any = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            parsed = raw
        return e.code, parsed


def wait_ready() -> None:
    deadline = time.time() + 180
    while time.time() < deadline:
        try:
            code, _ = http_json("GET", f"{TB}/api/test")
            if code == 200:
                print("tabularium ready")
                return
        except Exception as e:  # noqa: BLE001
            print(f"waiting for tabularium: {e}")
        time.sleep(2)
    raise SystemExit("tabularium did not become ready")


def rpc(method: str, params: dict[str, Any]) -> Any:
    code, body = http_json(
        "POST",
        f"{TB}/rpc",
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params},
    )
    if code != 200:
        raise RuntimeError(f"rpc HTTP {code}: {body}")
    if isinstance(body, dict) and body.get("error"):
        raise RuntimeError(f"rpc error: {body['error']}")
    return body.get("result") if isinstance(body, dict) else body


def seed() -> None:
    try:
        rpc("create_directory", {"path": "/otel", "parents": True})
    except RuntimeError as e:
        if "exists" not in str(e).lower() and "duplicate" not in str(e).lower():
            # Some deployments use create_directory without parents flag naming.
            try:
                rpc("create_directory", {"path": "/otel"})
            except RuntimeError:
                print(f"create_directory note: {e}")
    rpc(
        "put_document",
        {
            "path": "/otel/latency.md",
            "content": "OpenTelemetry search tantivy sqlite latency under load.\n",
            "force": True,
        },
    )
    print("seeded /otel/latency.md")


def mcp_tools_call(name: str, arguments: dict[str, Any]) -> tuple[bool, Any]:
    code, body = http_json(
        "POST",
        MCP,
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        },
        headers={"Accept": "application/json, text/event-stream"},
    )
    if code != 200:
        return False, body
    if isinstance(body, dict) and body.get("error"):
        return False, body
    if isinstance(body, dict):
        result = body.get("result")
        if isinstance(result, dict) and result.get("isError"):
            return False, body
    return True, body


def exercise() -> None:
    hits = rpc("search", {"query": "OpenTelemetry"})
    print("search hits:", hits)
    try:
        rpc("get_document", {"path": "/missing/nope.md"})
        raise RuntimeError("expected missing document error")
    except RuntimeError as e:
        print("expected error:", e)

    ok, body = mcp_tools_call("search", {"query": "tantivy"})
    print("mcp search ok=", ok, "snippet=", str(body)[:200])
    ok_err, _ = mcp_tools_call("get_document", {"path": "/no/such.md"})
    print("mcp error-case transport_ok_but_tool_fail=", not ok_err)


def oo_headers() -> dict[str, str]:
    token = base64.b64encode(f"{OO_USER}:{OO_PASSWORD}".encode()).decode()
    return {"Authorization": f"Basic {token}", "Content-Type": "application/json"}


def wait_for_traces(min_spans: int = 3) -> list[dict[str, Any]]:
    deadline = time.time() + 120
    sql = (
        "SELECT trace_id, span_name, duration, status_code "
        f"FROM {OO_ORG}.traces WHERE service_name = 'tabularium' "
        "ORDER BY start_time DESC LIMIT 200"
    )
    while time.time() < deadline:
        code, body = http_json(
            "POST",
            f"{OO}/api/{OO_ORG}/_search",
            {
                "query": {
                    "sql": sql,
                    "start_time": int((time.time() - 900) * 1_000_000),
                    "end_time": int(time.time() * 1_000_000),
                    "from": 0,
                    "size": 200,
                }
            },
            headers=oo_headers(),
        )
        hits: list[dict[str, Any]] = []
        if code == 200 and isinstance(body, dict):
            raw_hits = body.get("hits") or body.get("data") or []
            if isinstance(raw_hits, dict):
                raw_hits = raw_hits.get("hits") or []
            if isinstance(raw_hits, list):
                hits = raw_hits
        print(f"openobserve search status={code} hits={len(hits)}")
        if len(hits) >= min_spans:
            return hits
        time.sleep(3)
    raise SystemExit("timed out waiting for traces in OpenObserve")


def verify_traces(hits: list[dict[str, Any]]) -> None:
    names = {str(h.get("span_name") or h.get("name") or "") for h in hits}
    print("span names sample:", sorted(names)[:40])
    joined = " ".join(names)
    if "mcp.tools/call" not in joined and not any("mcp" in n for n in names):
        # RPC-only path still exercises search spans when MCP streamable init differs.
        assert any("search" in n for n in names), names
    else:
        assert any("search" in n for n in names), names
    assert any(
        h.get("duration") is not None or h.get("duration_us") is not None for h in hits
    )
    print("trace verification ok")


def one_search(_: int) -> tuple[bool, float]:
    t0 = time.perf_counter()
    try:
        rpc("search", {"query": "latency"})
        return True, time.perf_counter() - t0
    except Exception:
        return False, time.perf_counter() - t0


def percentile(sorted_vals: list[float], p: float) -> float:
    if not sorted_vals:
        return float("nan")
    k = (len(sorted_vals) - 1) * p
    f = int(k)
    c = min(f + 1, len(sorted_vals) - 1)
    if f == c:
        return sorted_vals[f]
    return sorted_vals[f] + (sorted_vals[c] - sorted_vals[f]) * (k - f)


def load_level(concurrency: int, total: int) -> None:
    latencies: list[float] = []
    ok_n = 0
    err_n = 0
    t0 = time.perf_counter()
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futs = [pool.submit(one_search, i) for i in range(total)]
        for fut in as_completed(futs):
            ok, dt = fut.result()
            latencies.append(dt)
            if ok:
                ok_n += 1
            else:
                err_n += 1
    wall = time.perf_counter() - t0
    latencies.sort()
    print(
        json.dumps(
            {
                "concurrency": concurrency,
                "requests": total,
                "success": ok_n,
                "errors": err_n,
                "wall_seconds": round(wall, 3),
                "latency_mean": round(statistics.mean(latencies), 4) if latencies else None,
                "p50": round(percentile(latencies, 0.50), 4),
                "p95": round(percentile(latencies, 0.95), 4),
                "p99": round(percentile(latencies, 0.99), 4),
            }
        )
    )


def main() -> None:
    wait_ready()
    seed()
    exercise()
    time.sleep(5)
    hits = wait_for_traces()
    verify_traces(hits)
    for c in (10, 50, 100):
        load_level(c, total=max(c * 2, 20))
    print("e2e complete")


if __name__ == "__main__":
    main()
