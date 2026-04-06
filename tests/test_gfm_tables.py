"""GFM pipe tables — integration with shared fixture (`meetings/tables`).

Requires TABULARIUM_TEST_URL and a running server (see conftest).
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import requests

from tests.helpers import _base, _slug, _tb_bin

FIXTURE_PATH = Path(__file__).resolve().parent / "fixtures" / "gfm_table_fixture.md"


def test_cli_cat_renders_gfm_fixture_body():
    """CLI `cat` renders markdown only on a TTY; captured output is raw (meetings/tables)."""
    base = _base()
    tb = _tb_bin()
    body = FIXTURE_PATH.read_text(encoding="utf-8")
    slug = _slug()
    rel = f"{slug}/gfm-table-fixture.md"
    r = requests.put(
        f"{base.rstrip('/')}/api/doc/{rel}",
        data=body.encode("utf-8"),
        headers={"Content-Type": "text/plain; charset=utf-8"},
        timeout=20,
    )
    r.raise_for_status()
    p = subprocess.run(
        [tb, "-u", base, "cat", f"/{rel}"],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert p.returncode == 0, p.stderr
    out = p.stdout
    assert "tokio" in out
    assert "Domain" in out
    assert "|--------|-----|-------|" in out, "captured stdout should keep raw GFM separator line"
