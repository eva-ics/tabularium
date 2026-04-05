import os
import subprocess
import tempfile
from pathlib import Path

import requests

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc

# Filesystem / JSON integer tolerance (see meetings/mtimes — Ferrum): ±2s in nanoseconds.
_MTIME_TOL_NS = 2_000_000_000


def _list_directory(dir_path: str) -> list:
    j = _rpc("list_directory", {"path": dir_path})
    assert j.get("error") is None, j
    return j["result"]


def _modified_at_for_name(dir_path: str, name: str) -> int:
    for row in _list_directory(dir_path):
        if row["name"] == name:
            return int(row["modified_at"])
    raise AssertionError(f"missing {name!r} under {dir_path!r}")


def _doc_modified_at_ns(path: str) -> int:
    j = _rpc("get_document_ref", {"path": path})
    assert j.get("error") is None, j
    return int(j["result"]["modified_at"])


def _assert_mtime_close_ns(got: int, want: int) -> None:
    assert abs(got - want) <= _MTIME_TOL_NS, (got, want)


def test_cli_put_stdin():
    tb = _tb_bin()
    base = _base()
    cat = f"py_cli_put_{_slug()}"
    doc = "note.md"
    mkdir(base, cat)
    path = f"{cat}/{doc}"
    body = "cogitator_payload"
    r = subprocess.run(
        [tb, "-u", base, "put", path],
        input=body,
        text=True,
        capture_output=True,
        timeout=20,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    gr = requests.get(f"{base}/api/doc/{cat}/{doc}", timeout=10)
    gr.raise_for_status()
    assert gr.json()["content"] == body
    body2 = "replaced_by_put_upsert"
    r2 = subprocess.run(
        [tb, "-u", base, "put", path],
        input=body2,
        text=True,
        capture_output=True,
        timeout=20,
        check=False,
    )
    assert r2.returncode == 0, r2.stderr
    gr2 = requests.get(f"{base}/api/doc/{cat}/{doc}", timeout=10)
    gr2.raise_for_status()
    assert gr2.json()["content"] == body2
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_import_root_dir():
    tb = _tb_bin()
    base = _base()
    cat_folder = f"py_imp_rf_{_slug()}"
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        (t / "root_only_file.txt").write_text("skip_me", encoding="utf-8")
        good = t / cat_folder
        good.mkdir()
        (good / "hello.txt").write_text("imported_body", encoding="utf-8")
        nested = good / "nested"
        nested.mkdir()
        (nested / "inner.txt").write_text("nope", encoding="utf-8")
        (good / "123").write_text("bad_decimal_name", encoding="utf-8")

        r = subprocess.run(
            [tb, "-u", base, "import", "/", tmp],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    assert r.returncode == 0, (r.stdout, r.stderr)
    out = r.stdout
    assert "root_only_file.txt" in out and "SKIPPED" in out
    assert "nested/inner.txt" in out and "OK" in out
    assert "123" in out and "SKIPPED" in out
    assert "hello.txt" in out and "OK" in out
    gr = requests.get(f"{base}/api/doc/{cat_folder}/hello.txt", timeout=10)
    gr.raise_for_status()
    assert gr.json()["content"] == "imported_body"
    gr2 = requests.get(f"{base}/api/doc/{cat_folder}/nested/inner.txt", timeout=10)
    gr2.raise_for_status()
    assert gr2.json()["content"] == "nope"
    _rpc("delete_directory", {"path": cat_folder, "recursive": True})


def test_cli_import_root_second_run_errors():
    tb = _tb_bin()
    base = _base()
    cat_folder = f"py_imp_dup_{_slug()}"
    with tempfile.TemporaryDirectory() as tmp:
        d = Path(tmp) / cat_folder
        d.mkdir()
        (d / "once.txt").write_text("v1", encoding="utf-8")
        r1 = subprocess.run(
            [tb, "-u", base, "import", "/", tmp],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert r1.returncode == 0, r1.stdout
        r2 = subprocess.run(
            [tb, "-u", base, "import", "/", tmp],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    assert r2.returncode != 0
    assert "ERROR" in r2.stdout
    _rpc("delete_directory", {"path": cat_folder, "recursive": True})


def test_cli_import_subdirectory():
    tb = _tb_bin()
    base = _base()
    cat = f"py_imp_cat_{_slug()}"
    mkdir(base, cat)
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        (t / "flat.txt").write_text("flat_content", encoding="utf-8")
        sub = t / "subdir"
        sub.mkdir()
        (sub / "nested.txt").write_text("x", encoding="utf-8")
        r = subprocess.run(
            [tb, "-u", base, "import", cat, tmp],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    assert r.returncode == 0, r.stdout
    assert "subdir/nested.txt" in r.stdout and "OK" in r.stdout
    assert "flat.txt" in r.stdout and "OK" in r.stdout
    gr = requests.get(f"{base}/api/doc/{cat}/flat.txt", timeout=10)
    gr.raise_for_status()
    assert gr.json()["content"] == "flat_content"
    gr2 = requests.get(f"{base}/api/doc/{cat}/subdir/nested.txt", timeout=10)
    gr2.raise_for_status()
    assert gr2.json()["content"] == "x"
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_import_single_file():
    tb = _tb_bin()
    base = _base()
    cat = f"py_imp_file_{_slug()}"
    mkdir(base, cat)
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".txt", delete=False, encoding="utf-8"
    ) as f:
        f.write("single_file_body")
        path = f.name
    try:
        r = subprocess.run(
            [tb, "-u", base, "import", cat, path],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    finally:
        Path(path).unlink(missing_ok=True)
    assert r.returncode == 0, r.stdout + r.stderr
    name = Path(path).name
    assert name in r.stdout and "OK" in r.stdout
    gr = requests.get(f"{base}/api/doc/{cat}/{name}", timeout=10)
    gr.raise_for_status()
    assert gr.json()["content"] == "single_file_body"
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_export_all_directories():
    tb = _tb_bin()
    base = _base()
    c1 = f"py_exp_all_a_{_slug()}"
    c2 = f"py_exp_all_b_{_slug()}"
    for c, doc, content in [(c1, "d1.txt", "one"), (c2, "d2.txt", "two")]:
        mkdir(base, c)
        put_doc(base, c, doc, content)
    with tempfile.TemporaryDirectory() as dest:
        r = subprocess.run(
            [tb, "-u", base, "export", "/", "-d", dest],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert r.returncode == 0, r.stdout + r.stderr
        d = Path(dest)
        assert (d / c1 / "d1.txt").read_text(encoding="utf-8") == "one"
        assert (d / c2 / "d2.txt").read_text(encoding="utf-8") == "two"
    for c in (c1, c2):
        _rpc("delete_directory", {"path": c, "recursive": True})


def test_cli_touch_set_mtime_and_rfc3339():
    tb = _tb_bin()
    base = _base()
    cat = f"py_touch_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "t.md", "x")
    want_ns = 1_700_000_000_000_000_000
    r = subprocess.run(
        [tb, "-u", base, "touch", f"{cat}/t.md", "1700000000"],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    got = _doc_modified_at_ns(f"/{cat}/t.md")
    _assert_mtime_close_ns(got, want_ns)
    r2 = subprocess.run(
        [
            tb,
            "-u",
            base,
            "touch",
            f"{cat}/t.md",
            "2026-03-14T12:00:00+00:00",
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r2.returncode == 0, r2.stderr
    want2 = 1_773_489_600_000_000_000
    got2 = _doc_modified_at_ns(f"/{cat}/t.md")
    _assert_mtime_close_ns(got2, want2)
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_touch_invalid_time_does_not_change_mtime():
    tb = _tb_bin()
    base = _base()
    cat = f"py_touch_bad_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "t.md", "x")
    before = _doc_modified_at_ns(f"/{cat}/t.md")
    r = subprocess.run(
        [tb, "-u", base, "touch", f"{cat}/t.md", "not-a-date-at-all"],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode != 0, r.stdout
    after = _doc_modified_at_ns(f"/{cat}/t.md")
    assert after == before
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_import_preserves_file_mtime():
    tb = _tb_bin()
    base = _base()
    cat = f"py_m_imp_{_slug()}"
    mkdir(base, cat)
    want_sec = 1_700_000_000
    want_ns = want_sec * 1_000_000_000
    with tempfile.NamedTemporaryFile(
        mode="w", suffix=".txt", delete=False, encoding="utf-8"
    ) as f:
        f.write("mtime_body")
        path = f.name
    try:
        os.utime(path, (want_sec, want_sec))
        r = subprocess.run(
            [tb, "-u", base, "import", cat, path],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    finally:
        Path(path).unlink(missing_ok=True)
    assert r.returncode == 0, r.stdout + r.stderr
    name = Path(path).name
    got = _modified_at_for_name(f"/{cat}", name)
    _assert_mtime_close_ns(got, want_ns)
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_import_preserves_directory_mtime():
    tb = _tb_bin()
    base = _base()
    cat = f"py_m_d_imp_{_slug()}"
    mkdir(base, cat)
    want_sec = 1_701_000_000
    want_ns = want_sec * 1_000_000_000
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        sub = root / "nested"
        sub.mkdir()
        (sub / "inner.txt").write_text("in", encoding="utf-8")
        os.utime(sub, (want_sec, want_sec))
        r = subprocess.run(
            [tb, "-u", base, "import", cat, str(root)],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert r.returncode == 0, r.stdout + r.stderr
        got = _modified_at_for_name(f"/{cat}", "nested")
        _assert_mtime_close_ns(got, want_ns)
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_export_sets_local_directory_mtime():
    tb = _tb_bin()
    base = _base()
    cat = f"py_m_expd_{_slug()}"
    mkdir(base, cat)
    requests.post(
        f"{base}/api/doc",
        json={"path": f"/{cat}/sub", "description": None},
        timeout=10,
    ).raise_for_status()
    put_doc(base, f"{cat}/sub", "inner.txt", "d")
    want_ns = 1_703_000_000_000_000_000
    _rpc(
        "touch_document",
        {"path": f"/{cat}/sub", "modified_at": want_ns},
    )
    with tempfile.TemporaryDirectory() as dest:
        r = subprocess.run(
            [tb, "-u", base, "export", cat, "-d", dest],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert r.returncode == 0, r.stdout + r.stderr
        p = Path(dest) / "sub"
        st = p.stat()
        got_ns = getattr(st, "st_mtime_ns", int(st.st_mtime * 1e9))
        _assert_mtime_close_ns(got_ns, want_ns)
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_export_sets_local_file_mtime():
    tb = _tb_bin()
    base = _base()
    cat = f"py_m_exp_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "out.txt", "export_m")
    want_ns = 1_702_000_000_000_000_000
    _rpc(
        "touch_document",
        {"path": f"/{cat}/out.txt", "modified_at": want_ns},
    )
    with tempfile.TemporaryDirectory() as dest:
        r = subprocess.run(
            [tb, "-u", base, "export", cat, "-d", dest],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert r.returncode == 0, r.stdout + r.stderr
        p = Path(dest) / "out.txt"
        st = p.stat()
        got_ns = getattr(st, "st_mtime_ns", int(st.st_mtime * 1e9))
        _assert_mtime_close_ns(got_ns, want_ns)
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_export_single_directory_flat():
    tb = _tb_bin()
    base = _base()
    cat = f"py_exp_flat_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "solo.txt", "flat_export")
    with tempfile.TemporaryDirectory() as dest:
        r = subprocess.run(
            [tb, "-u", base, "export", cat, "-d", dest],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        assert r.returncode == 0, r.stdout + r.stderr
        assert (Path(dest) / "solo.txt").read_text(encoding="utf-8") == "flat_export"
    _rpc("delete_directory", {"path": cat, "recursive": True})
