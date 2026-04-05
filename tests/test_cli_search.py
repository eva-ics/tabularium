import subprocess

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_cli_search_snippet_has_no_raw_newlines():
    tb = _tb_bin()
    base = _base()
    cat = f"py_snip_nl_{_slug()}"
    token = f"py_snip_tok_{_slug()}"
    mkdir(base, cat)
    put_doc(
        base,
        cat,
        "body.md",
        f"aaa\nbbb\nccc {token} ddd\n",
    )
    r = subprocess.run(
        [tb, "-u", base, "search", token],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert r.returncode == 0, r.stderr
    seen = False
    for ln in r.stdout.splitlines():
        if token not in ln:
            continue
        seen = True
        parts = ln.split("\t")
        assert len(parts) >= 3
        snippet = parts[2]
        assert "\n" not in snippet
        assert "\r" not in snippet
    assert seen, r.stdout
    _rpc("delete_directory", {"path": cat, "recursive": True})
