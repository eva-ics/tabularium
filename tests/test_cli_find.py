import subprocess

import requests

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_cli_find_hits_and_exact_order():
    tb = _tb_bin()
    base = _base()
    tok = f"xftok{_slug()}"
    wrap = f"pre_{tok}_post"
    for name in (wrap, tok):
        mkdir(base, name)
    doc_name = f"{tok}.txt"
    put_doc(base, wrap, doc_name, "x")
    r = subprocess.run(
        [tb, "-u", base, "find", tok],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    lines = [ln for ln in r.stdout.splitlines() if ln.strip()]
    # Plain find: kind, path, modified_at (RFC3339).
    assert any(
        ln.startswith("directory\t") and ln.split("\t")[1] == f"/{tok}" for ln in lines
    )
    assert any("file\t" in ln and doc_name in ln for ln in lines)
    dir_lines = [ln for ln in lines if ln.startswith("directory\t")]
    assert dir_lines[0].split("\t")[1] == f"/{tok}"
    for name in (wrap, tok):
        _rpc("delete_directory", {"path": name, "recursive": True})


def test_cli_find_empty_name_errors():
    tb = _tb_bin()
    base = _base()
    r = subprocess.run(
        [tb, "-u", base, "find", ""],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r.returncode != 0


def test_cli_find_directory_scope_lists_doc():
    tb = _tb_bin()
    base = _base()
    cat = f"py_cli_find_c_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "scoped.md", "x")
    r = subprocess.run(
        [tb, "-u", base, "find", "-d", cat],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    assert "file" in r.stdout and "scoped.md" in r.stdout
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_find_scoped_without_needle_lists_subdirs_and_files():
    tb = _tb_bin()
    base = _base()
    parent = f"py_find_nd_{_slug()}"
    mkdir(base, parent)
    requests.post(
        f"{base}/api/doc",
        json={"path": f"/{parent}/inner", "description": None},
        timeout=10,
    ).raise_for_status()
    put_doc(base, parent, "root.md", "x")
    r = subprocess.run(
        [tb, "-u", base, "find", "-d", parent],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    assert f"/{parent}/inner" in r.stdout
    assert "directory\t" in r.stdout
    assert "root.md" in r.stdout and "file\t" in r.stdout
    _rpc("delete_directory", {"path": parent, "recursive": True})
