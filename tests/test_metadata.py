"""JSON metadata contracts, CLI sidecars, and conflict handling."""
import json
import os
import subprocess
import requests
from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_metadata_roundtrip_search_cli_and_validation(tmp_path):
    base = _base()
    root = "meta_" + _slug()
    mkdir(base, root)
    path = "/" + root + "/note.md"
    put_doc(base, root, "note.md", "source body")
    original = _rpc("get_document_metadata", {"path": path})["result"]
    assert original["metadata"] == {}
    needle = "metaneedle" + _slug()
    metadata = {"tags": [needle, "Кира"], "status": "draft", "nested": {"n": 42, "ok": True}}
    params = {"path": path, "metadata": metadata, "only_if_revision": original["revision"]}
    assert "result" in _rpc("set_document_metadata", params)
    assert "error" in _rpc("set_document_metadata", params)
    rest = requests.put(base + "/api/metadata" + path, json={"metadata": metadata}, timeout=10)
    rest.raise_for_status()
    assert requests.put(base + "/api/metadata" + path, json={"metadata": {}, "only_if_revision": original["revision"]}, timeout=10).status_code == 409
    assert _rpc("get_document", {"path": path})["result"]["content"] == "source body"
    response = requests.get(base + "/api/metadata" + path, timeout=10)
    response.raise_for_status()
    assert response.json()["metadata"] == metadata
    assert requests.get(base + "/api/doc" + path, timeout=10).json()["metadata"] == metadata
    hits = _rpc("search", {"query": needle, "kind": "lexical", "path": "/" + root})["result"]
    assert [h["path"] for h in hits] == [path]
    for bad in [[], None, "oops", {"large": "x" * 65536}]:
        assert "error" in _rpc("set_document_metadata", {"path": path, "metadata": bad})
        assert requests.put(base + "/api/metadata" + path, json={"metadata": bad}, timeout=10).status_code == 400
    assert "error" in _rpc("get_document_metadata", {"path": "/" + root})
    def cli(*args, env=None):
        result = subprocess.run([_tb_bin(), "-u", base, *args], capture_output=True, text=True, env=env)
        assert result.returncode == 0, result.stderr + result.stdout
        return result.stdout
    out = tmp_path / "export"
    out.mkdir()
    cli("export", "/" + root, "-d", str(out))
    sidecar = out / "note.md.metadata.json"
    assert json.loads(sidecar.read_text()) == metadata
    destination = "meta_import_" + _slug()
    cli("import", "/" + destination, str(out))
    imported = "/" + destination + "/note.md"
    assert json.loads(cli("meta", imported)) == metadata
    entries = _rpc("list_directory", {"path": "/" + destination})["result"]
    assert len(entries) == 1  # sidecar is not an independent document
    explicit = tmp_path / "explicit.json"
    cli("meta", imported, "--export", str(explicit))
    assert json.loads(explicit.read_text()) == metadata
    explicit.write_text('{"new": "freshmetadata"}')
    cli("meta", imported, "--import", str(explicit))
    editor = tmp_path / "editor.sh"
    editor.write_text("#!/bin/sh\nprintf '{\"edited\":true}\\n' > \"$1\"\n")
    editor.chmod(0o700)
    cli("meta", imported, "--edit", env=dict(os.environ, EDITOR=str(editor)))
    assert json.loads(cli("meta", imported)) == {"edited": True}
    # Invalid sidecar must fail before creating the paired document.
    bad_source = tmp_path / "broken.md"
    bad_source.write_text("body")
    (tmp_path / "broken.md.metadata.json").write_text("[]")
    failed = subprocess.run([_tb_bin(), "-u", base, "import", "/" + destination, str(bad_source)], capture_output=True)
    assert failed.returncode != 0
    assert not _rpc("exists", {"path": "/" + destination + "/broken.md"})["result"]

    # Single-file rename still looks for the sidecar beside the original filename.
    cli("import", "/" + destination, str(out / "note.md"), "--name", "renamed.md")
    assert json.loads(cli("meta", "/" + destination + "/renamed.md")) == metadata
    _rpc("put_document", {"path": imported, "content": "edited body", "force": True})
    assert json.loads(cli("meta", imported)) == {"edited": True}
    # Removing metadata removes old tokens from lexical search.
    requests.put(base + "/api/metadata" + imported, json={"metadata": {}}, timeout=10).raise_for_status()
    assert not _rpc("search", {"query": "freshmetadata", "kind": "lexical", "path": imported})["result"]
    empty_export = tmp_path / "empty_export"
    empty_export.mkdir()
    cli("export", "/" + destination, "-d", str(empty_export))
    assert not (empty_export / "note.md.metadata.json").exists()
    # Reserved sidecar names must fail visibly, not overwrite metadata or vanish on reimport.
    put_doc(base, destination, "renamed.md.metadata.json", "ordinary document")
    collision_export = tmp_path / "collision_export"
    collision_export.mkdir()
    collision = subprocess.run([_tb_bin(), "-u", base, "export", "/" + destination, "-d", str(collision_export)], capture_output=True)
    assert collision.returncode != 0

    # Pretty-printed sidecars can exceed the canonical JSON size limit due to whitespace.
    large = {"tags": ["x"] * 9000}
    explicit.write_text(json.dumps(large, separators=(",", ":")))
    cli("meta", imported, "--import", str(explicit))
    pretty_large = tmp_path / "large-pretty.json"
    cli("meta", imported, "--export", str(pretty_large))
    assert pretty_large.stat().st_size > 65536
    cli("meta", imported, "--import", str(pretty_large))
    assert json.loads(cli("meta", imported)) == large
