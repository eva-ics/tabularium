import requests

from tests.helpers import _base, _rpc, _slug, mkdir, put_doc


def test_text_ops_via_rpc():
    base = _base()
    cat = "py_txt_cat"
    path = f"{cat}/py_txt_doc"
    mkdir(base, cat)
    put_doc(base, cat, "py_txt_doc", "a\nb\nc")

    h = _rpc("head", {"path": path, "lines": 2})
    assert h["result"]["text"] == "a\nb"

    g = _rpc("grep", {"path": path, "pattern": "b", "max_matches": 5})
    assert g["result"][0]["line"] == 2


def test_rpc_append_creates_missing_document():
    base = _base()
    cat = "py_append_create_cat"
    mkdir(base, cat)
    out = _rpc(
        "append_document",
        {"path": f"{cat}/py_new_append_doc", "content": "segmentum"},
    )
    assert "result" in out
    r = requests.get(f"{base}/api/doc/{cat}/py_new_append_doc", timeout=10)
    r.raise_for_status()
    assert r.json()["content"] == "segmentum"


def test_rpc_append_if_not_contains_roundtrip():
    base = _base()
    cat = "py_ainc_cat"
    path = f"{cat}/py_ainc_doc"
    mkdir(base, cat)
    put_doc(base, cat, "py_ainc_doc", "alpha")

    r1 = _rpc(
        "append_if_not_contains",
        {"path": path, "marker": "OMEGA", "content": "\nOMEGA\n"},
    )
    assert r1.get("result", {}).get("appended") is True, r1

    r2 = _rpc(
        "append_if_not_contains",
        {"path": path, "marker": "OMEGA", "content": "\nOMEGA\n"},
    )
    assert r2.get("result", {}).get("appended") is False, r2

    r = requests.get(f"{base}/api/doc/{cat}/py_ainc_doc", timeout=10)
    r.raise_for_status()
    body = r.json()["content"]
    assert body.count("OMEGA") == 1


def test_rpc_append_if_not_contains_missing_document_errors():
    base = _base()
    cat = "py_ainc_miss"
    mkdir(base, cat)
    out = _rpc(
        "append_if_not_contains",
        {"path": f"{cat}/no_such_file", "marker": "x", "content": "y"},
    )
    assert "error" in out
    assert out["error"]["code"] == -32603


def test_rpc_append_if_not_contains_empty_marker_invalid():
    base = _base()
    cat = "py_ainc_badmk"
    path = f"{cat}/py_ainc_badmk_doc"
    mkdir(base, cat)
    put_doc(base, cat, "py_ainc_badmk_doc", "z")
    out = _rpc(
        "append_if_not_contains",
        {"path": path, "marker": "", "content": "y"},
    )
    assert "error" in out
    assert out["error"]["code"] == -32602


def test_rpc_create_directory_parents_false_requires_parent():
    _base()
    s = _slug()
    o = _rpc("create_directory", {"path": f"/gap_{s}/child"})
    assert "error" in o


def test_rpc_create_directory_parents_true_and_idempotent():
    _base()
    s = _slug()
    root = f"p_mk_{s}"
    p = f"/{root}/x/y"
    r1 = _rpc("create_directory", {"path": p, "parents": True})
    assert "result" in r1, r1
    r2 = _rpc("create_directory", {"path": p, "parents": True})
    assert r1["result"] == r2["result"]


def test_rpc_create_directory_omitted_parents_defaults_strict():
    base = _base()
    s = _slug()
    o = _rpc("create_directory", {"path": f"/hole_{s}/deep/nested"})
    assert "error" in o


def test_rpc_delete_directory_recursive():
    base = _base()
    cat = "py_rec_rm_cat"
    mkdir(base, cat)
    put_doc(base, cat, "held", "z")
    out = _rpc("delete_directory", {"path": cat, "recursive": True})
    assert "result" in out
    r = requests.get(f"{base}/api/doc", timeout=10)
    r.raise_for_status()
    names = [row["name"] for row in r.json()]
    assert cat not in names
