"""Force-guard for put_document / append_document (JSON-RPC + CLI).

Default safe (`force` omitted or false): existing target → Duplicate (-32002).
`force=true`: legacy upsert (put replaces, append appends).
say_document remains exempt (cannot create new scrolls).
"""

import subprocess

import requests

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir

DUPLICATE_RESOURCE = -32002


def _put(path: str, content: str, **extra) -> dict:
    params = {"path": path, "content": content}
    params.update(extra)
    return _rpc("put_document", params)


def _append(path: str, content: str, **extra) -> dict:
    params = {"path": path, "content": content}
    params.update(extra)
    return _rpc("append_document", params)


def _get_content(base: str, rel_path: str) -> str:
    r = requests.get(f"{base}/api/doc/{rel_path}", timeout=10)
    r.raise_for_status()
    return r.json()["content"]


def test_put_document_default_force_false_rejects_existing():
    base = _base()
    cat = f"fg_put_def_{_slug()}"
    mkdir(base, cat)

    r1 = _put(f"/{cat}/d", "first")
    assert "result" in r1, r1

    r2 = _put(f"/{cat}/d", "second")
    assert r2.get("error", {}).get("code") == DUPLICATE_RESOURCE, r2

    assert _get_content(base, f"{cat}/d") == "first"

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_put_document_force_true_overwrites():
    base = _base()
    cat = f"fg_put_force_{_slug()}"
    mkdir(base, cat)

    _put(f"/{cat}/d", "first")
    r = _put(f"/{cat}/d", "OVERWRITE", force=True)
    assert "result" in r, r

    assert _get_content(base, f"{cat}/d") == "OVERWRITE"

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_append_document_default_force_false_rejects_existing():
    base = _base()
    cat = f"fg_app_def_{_slug()}"
    mkdir(base, cat)

    r1 = _append(f"/{cat}/d", "alpha")
    assert "result" in r1, r1

    r2 = _append(f"/{cat}/d", "beta")
    assert r2.get("error", {}).get("code") == DUPLICATE_RESOURCE, r2

    assert _get_content(base, f"{cat}/d") == "alpha"

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_append_document_force_true_appends_not_replaces():
    base = _base()
    cat = f"fg_app_force_{_slug()}"
    mkdir(base, cat)

    _put(f"/{cat}/d", "head")
    r = _append(f"/{cat}/d", "tail", force=True)
    assert "result" in r, r

    # append() inserts a single newline boundary when needed.
    assert _get_content(base, f"{cat}/d") == "head\ntail"

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_say_document_still_errors_on_missing_target():
    """Regression guard for the documented `say_document` exemption."""
    base = _base()
    cat = f"fg_say_{_slug()}"
    mkdir(base, cat)

    r = _rpc(
        "say_document",
        {
            "path": f"/{cat}/no_such_doc",
            "from_id": "Cogis",
            "content": "ave omnissiah",
        },
    )
    assert "error" in r, r

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_force_accepts_string_true_and_false():
    """Some clients send booleans as strings — extractor must accept them."""
    base = _base()
    cat = f"fg_str_{_slug()}"
    mkdir(base, cat)

    _put(f"/{cat}/d", "first")
    r = _put(f"/{cat}/d", "second", force="true")
    assert "result" in r, r
    assert _get_content(base, f"{cat}/d") == "second"

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_put_default_rejects_existing_and_force_overwrites():
    tb = _tb_bin()
    base = _base()
    cat = f"fg_cli_put_{_slug()}"
    path = f"/{cat}/d"
    mkdir(base, cat)

    # First put creates.
    r1 = subprocess.run(
        [tb, "-u", base, "put", path],
        input="first",
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r1.returncode == 0, r1.stderr

    # Second put without --force fails.
    r2 = subprocess.run(
        [tb, "-u", base, "put", path],
        input="second",
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r2.returncode != 0, "expected failure without --force"
    assert _get_content(base, f"{cat}/d") == "first"

    # Third put with --force overwrites.
    r3 = subprocess.run(
        [tb, "-u", base, "put", "--force", path],
        input="OVERWRITE",
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r3.returncode == 0, r3.stderr
    assert _get_content(base, f"{cat}/d") == "OVERWRITE"

    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_append_default_rejects_existing_and_force_appends():
    tb = _tb_bin()
    base = _base()
    cat = f"fg_cli_app_{_slug()}"
    path = f"/{cat}/d"
    mkdir(base, cat)

    r1 = subprocess.run(
        [tb, "-u", base, "append", path],
        input="head",
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r1.returncode == 0, r1.stderr

    r2 = subprocess.run(
        [tb, "-u", base, "append", path],
        input="more",
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r2.returncode != 0, "expected failure without --force"
    assert _get_content(base, f"{cat}/d") == "head"

    r3 = subprocess.run(
        [tb, "-u", base, "append", "-f", path],
        input="tail",
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r3.returncode == 0, r3.stderr
    assert _get_content(base, f"{cat}/d") == "head\ntail"

    _rpc("delete_directory", {"path": cat, "recursive": True})
