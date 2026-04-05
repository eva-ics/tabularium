"""Shared helpers for optional integration tests (TABULARIUM_TEST_URL)."""

from __future__ import annotations

import os
import shutil
import uuid

import pytest
import requests


def _base() -> str:
    return os.environ["TABULARIUM_TEST_URL"].rstrip("/")


def _rpc(method: str, params: dict) -> dict:
    r = requests.post(
        f"{_base()}/rpc",
        json={
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        },
        timeout=10,
    )
    r.raise_for_status()
    return r.json()


def _tb_bin() -> str:
    tb = os.environ.get("TABULARIUM_TB_BIN") or shutil.which("tb")
    if not tb:
        pytest.skip("set TABULARIUM_TB_BIN or install tb on PATH")
    return tb


def _slug() -> str:
    return uuid.uuid4().hex[:8]


def mkdir(base: str, name: str) -> None:
    """Create top-level directory `/name` via REST."""
    requests.post(
        f"{base}/api/doc",
        json={"path": f"/{name}", "description": None},
        timeout=10,
    ).raise_for_status()


def put_doc(base: str, cat: str, doc: str, content: str) -> None:
    requests.put(
        f"{base}/api/doc/{cat}/{doc}",
        json={"content": content},
        timeout=10,
    ).raise_for_status()
