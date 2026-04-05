import requests

from tests.helpers import _base, _rpc


def test_name_validation():
    base = _base()
    r = requests.post(
        f"{base}/api/doc",
        json={"path": "/bad//slash", "description": None},
        timeout=10,
    )
    assert r.status_code == 400

    err = _rpc("create_directory", {"path": r"x\y", "description": None})
    assert err.get("error", {}).get("code") == -32602
