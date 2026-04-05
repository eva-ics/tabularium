import requests

from tests.helpers import _base, _rpc, mkdir, put_doc


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
