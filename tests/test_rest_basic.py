import requests

from tests.helpers import _base, _rpc


def test_get_api_doc_root_ok():
    r = requests.get(f"{_base()}/api/doc", timeout=5)
    assert r.status_code == 200
    assert isinstance(r.json(), list)


def test_rpc_list_directory():
    data = _rpc("list_directory", {})
    assert "result" in data


def test_crud_roundtrip():
    base = _base()
    cat = "py_crud_cat"
    doc = "py_crud_doc"

    r = requests.post(
        f"{base}/api/doc",
        json={"path": f"/{cat}", "description": None},
        timeout=10,
    )
    assert r.status_code == 201
    assert "Location" in r.headers

    r = requests.put(
        f"{base}/api/doc/{cat}/{doc}",
        json={"content": "alpha"},
        timeout=10,
    )
    assert r.status_code == 204

    r = requests.get(f"{base}/api/doc/{cat}/{doc}", timeout=10)
    assert r.status_code == 200
    assert r.json()["content"] == "alpha"

    r = requests.delete(f"{base}/api/doc/{cat}/{doc}", timeout=10)
    assert r.status_code == 204

    r = requests.get(f"{base}/api/doc/{cat}/{doc}", timeout=10)
    assert r.status_code == 404
