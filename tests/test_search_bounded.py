"""P1: bounded source-addressable search and revision-checked reads."""
import json
import subprocess

import requests

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_search_bounds_source_revision_and_legacy_shape():
    base = _base()
    root = 'bounded_' + _slug()
    word = 'boundedneedle' + _slug()
    mkdir(base, root)
    body = ('prefix line\n' * 20) + word + ' café 日本語\nlast line\n'
    for name in ['a.txt', 'b.txt', 'c.txt']:
        put_doc(base, root, name, body)
    params = dict(query=word, path='/' + root, kind='lexical', limit=1)
    report = _rpc('search', params)['result']
    assert len(report['hits']) == 1
    assert report['has_more'] is True and report['truncated'] is True
    hit = report['hits'][0]
    assert hit['revision'] and isinstance(hit['modified_at'], int)
    assert hit['start_line'] <= hit['line_number'] <= hit['end_line']
    assert hit['end_line'] == hit['line_end']
    assert hit['snippet'] in body and word in hit['snippet']
    again = _rpc('search', params)['result']
    assert again['hits'] == report['hits']
    slice_params = dict(path=hit['path'], start_line=hit['start_line'], end_line=hit['end_line'], expected_revision=hit['revision'])
    source = _rpc('slice', slice_params)['result']
    assert source['revision'] == hit['revision']
    assert hit['snippet'].rstrip('\n') in source['text']
    _rpc('put_document', dict(path=hit['path'], content='replacement', force=True))
    stale = _rpc('slice', slice_params)
    assert stale['error']['code'] == -32003
    legacy = _rpc('search', dict(query=word, path='/' + root, kind='lexical'))['result']
    assert isinstance(legacy, list)
    report = _rpc('search', dict(params, limit=256))['result']
    assert len(report['hits']) == 2
    assert report['has_more'] is False


def test_search_utf8_budget_rest_rpc_forms_and_cli():
    base = _base()
    root = 'budget_' + _slug()
    word = 'budgetneedle' + _slug()
    mkdir(base, root)
    for i in range(6):
        put_doc(base, root, str(i) + '.txt', word + '\n' + ('"\\日本語' * 300))
    payload = dict(q=word, dir='/' + root, kind='lexical', limit=5, max_output_bytes=1500)
    response = requests.get(base + '/api/search', params=payload, timeout=10)
    response.raise_for_status()
    assert len(response.content) <= 1500
    report = response.json()
    assert report['truncated'] is True
    assert response.headers['x-tabularium-search-truncated'] == 'true'
    rpc_params = dict(query=word, path='/' + root, kind='lexical', limit=5, max_output_bytes=1500)
    rpc = _rpc('search', rpc_params)['result']
    assert len(json.dumps(rpc, ensure_ascii=False, separators=(',', ':')).encode()) <= 1500
    assert rpc == report
    for mode in ['json', 'data']:
        posted = requests.post(base + '/api/search', **{mode: payload}, timeout=10)
        posted.raise_for_status()
        assert posted.json() == report
    result = subprocess.run([_tb_bin(), '-u', base, 'search', '--kind', 'lexical', '-d', '/' + root, '--limit', '5', '--max-output-bytes', '1500', '--json', word], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)['truncated'] is True
    for limits in [dict(limit=0), dict(limit=257), dict(max_output_bytes=511), dict(max_output_bytes=1048577), dict(limit='oops')]:
        error = _rpc('search', dict(query=word, **limits))
        assert error['error']['code'] == -32602
