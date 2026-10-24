import subprocess

import requests

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_cli_wc_glob_lines_words_bytes():
    tb = _tb_bin()
    base = _base()
    cat = f"py_wc_glob_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "a.md", "one two\nthree")
    put_doc(base, cat, "b.md", "four\nfive six")
    for flag, expect_total in (("-l", "4"), ("-w", "6"), ("-c", "26")):
        r = subprocess.run(
            [tb, "-u", base, "wc", flag, f"/{cat}/*"],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        assert r.returncode == 0, r.stderr
        assert "total" in r.stdout
        # GNU-style line is `{count} total` — count sits before the label.
        total_line = next(line for line in r.stdout.splitlines() if line.endswith(" total"))
        assert expect_total in total_line.split()
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_wc_directory_errors():
    tb = _tb_bin()
    base = _base()
    cat = f"py_wc_dir_{_slug()}"
    mkdir(base, cat)
    r = subprocess.run(
        [tb, "-u", base, "wc", "-l", f"/{cat}"],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode != 0
    assert "is a directory" in (r.stderr + r.stdout)
    assert "recursive" in (r.stderr + r.stdout)
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_wc_recursive_glob():
    tb = _tb_bin()
    base = _base()
    cat = f"py_wc_rec_{_slug()}"
    mkdir(base, cat)
    requests.post(
        f"{base}/api/doc",
        json={"path": f"/{cat}/inner", "description": None},
        timeout=10,
    ).raise_for_status()
    put_doc(base, f"{cat}/inner", "a.md", "x\ny")
    r = subprocess.run(
        [tb, "-u", base, "wc", "-l", "-r", f"/{cat}/*"],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    # One matched file may print the single-column form (`2`) without a `total` row.
    if "total" in r.stdout:
        assert "2" in r.stdout
    else:
        assert r.stdout.strip() == "2"
    _rpc("delete_directory", {"path": cat, "recursive": True})


def test_cli_wc_single_file_lines_only():
    tb = _tb_bin()
    base = _base()
    cat = f"py_wc_one_{_slug()}"
    mkdir(base, cat)
    put_doc(base, cat, "solo.md", "a\nb\nc")
    r = subprocess.run(
        [tb, "-u", base, "wc", "-l", f"/{cat}/solo.md"],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == "3"
    _rpc("delete_directory", {"path": cat, "recursive": True})
