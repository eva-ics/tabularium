"""Temporal search contracts through real REST, RPC, MCP and CLI transports."""
import json
import subprocess

import requests

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_time_order_exclusive_bounds_transports():
    base = _base()
    root = "time_" + _slug()
    word = "timeneedle" + _slug()
    mkdir(base, root)
    stamps = []
    for name in ["z.txt", "b.txt", "a.txt"]:
        put_doc(base, root, name, word)
        stamps.append(_rpc("get_document", {"path": "/" + root + "/" + name})["result"]["modified_at"])
    assert stamps == sorted(set(stamps))
    common = dict(query=word, path="/" + root, kind="lexical", limit=1, sort="modified_desc")
    report = _rpc("search", common)["result"]
    assert report["hits"][0]["path"].endswith("/a.txt")
    assert report["has_more"]
    filtered = dict(common, modified_after=stamps[0], modified_before=stamps[2])
    expected = _rpc("search", filtered)["result"]
    assert [h["path"] for h in expected["hits"]] == ["/" + root + "/b.txt"]
    assert not expected["has_more"]
    assert not _rpc("search", dict(common, modified_after=stamps[2]))["result"]["hits"]
    assert not _rpc("search", dict(common, modified_before=stamps[0]))["result"]["hits"]
    payload = {("q" if k == "query" else "dir" if k == "path" else k): v for k, v in filtered.items()}
    got = requests.get(base + "/api/search", params=payload, timeout=10)
    got.raise_for_status()
    assert got.json() == expected
    for mode in ["json", "data"]:
        got = requests.post(base + "/api/search", **{mode: payload}, timeout=10)
        got.raise_for_status()
        assert got.json() == expected
    got = requests.post(base + "/api/search", files={k: (None, str(v)) for k, v in payload.items()}, timeout=10)
    got.raise_for_status()
    assert got.json() == expected
    cli = subprocess.run([_tb_bin(), "-u", base, "search", "--kind", "lexical", "-d", "/" + root,
        "--sort", "modified_desc", "--modified-after", str(stamps[0]),
        "--modified-before", str(stamps[2]), "--limit", "1", "--json", word],
        capture_output=True, text=True)
    assert cli.returncode == 0, cli.stderr
    cli_report = json.loads(cli.stdout)
    assert [h["path"] for h in cli_report["hits"]] == [h["path"] for h in expected["hits"]]
    assert cli_report["has_more"] == expected["has_more"]
    for key in ["revision", "modified_at", "start_line", "line_end", "snippet"]:
        assert cli_report["hits"][0][key] == expected["hits"][0][key]
    # Existing array-returning shape remains unchanged when no envelope is requested.
    assert isinstance(_rpc("search", {k: v for k, v in common.items() if k != "limit"})["result"], list)
    for bad in [dict(sort="oops"), dict(modified_after=-1), dict(modified_before="yesterday"),
                dict(modified_after=True), dict(modified_after=2, modified_before=1)]:
        assert _rpc("search", dict(common, **bad))["error"]["code"] == -32602
        rest = {("q" if k == "query" else "dir" if k == "path" else k): v for k, v in dict(common, **bad).items()}
        assert requests.post(base + "/api/search", json=rest, timeout=10).status_code == 400
