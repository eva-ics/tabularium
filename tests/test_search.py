import urllib.parse

import requests

from tests.helpers import _base, mkdir, put_doc


def test_search():
    base = _base()
    cat = "py_search_cat"
    needle = "pyuniqneedle"
    mkdir(base, cat)
    put_doc(base, cat, "sdoc", f"hello {needle} world")

    q = urllib.parse.quote(needle)
    r = requests.get(f"{base}/api/search?q={q}", timeout=10)
    assert r.status_code == 200
    hits = r.json()
    assert len(hits) >= 1
    assert any(needle.lower() in h["snippet"].lower() for h in hits)
