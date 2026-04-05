import signal
import subprocess
import time

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def test_cli_wait_respects_client_timeout():
    """`tb` uses one reqwest timeout; wait long-polls — short `-t` must fail before server ends."""
    tb = _tb_bin()
    base = _base()
    cat = "py_cli_wait_cat"
    doc = "py_cli_wait_doc"
    mkdir(base, cat)
    put_doc(base, cat, doc, "idle")
    path = f"{cat}/{doc}"
    r = subprocess.run(
        [tb, "-t", "1", "-u", base, "wait", path],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r.returncode != 0


def test_cli_wait_ctrl_c_interrupted_and_exit_130():
    tb = _tb_bin()
    base = _base()
    cat = f"py_wait_sig_{_slug()}"
    doc = "hold.md"
    mkdir(base, cat)
    put_doc(base, cat, doc, "block")
    path = f"{cat}/{doc}"
    proc = subprocess.Popen(
        [tb, "-u", base, "-t", "120", "wait", path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    time.sleep(0.8)
    proc.send_signal(signal.SIGINT)
    out, _err = proc.communicate(timeout=30)
    assert proc.returncode == 130
    assert "interrupted" in out
    _rpc("delete_directory", {"path": cat, "recursive": True})
