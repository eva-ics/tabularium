import os
import pty
import select
import signal
import subprocess
import sys
import time

import pytest

from tests.helpers import _base, _rpc, _slug, _tb_bin, mkdir, put_doc


def _pty_drain(fd: int, buf: bytearray, timeout: float) -> None:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        rem = end - time.monotonic()
        if rem <= 0:
            break
        r, _, _ = select.select([fd], [], [], min(0.25, rem))
        if not r:
            continue
        try:
            chunk = os.read(fd, 8192)
        except OSError:
            break
        if not chunk:
            break
        buf.extend(chunk)


@pytest.mark.skipif(
    sys.platform.startswith("win"),
    reason="shell wait PTY/SIGINT ritual is unix-only",
)
def test_shell_wait_ctrl_c_then_exit_clean():
    """Regression: shell `wait` must not leave tokio SIGINT state that surfaces after `exit`."""
    tb = _tb_bin()
    base = _base()
    cat = f"py_sh_wait_{_slug()}"
    doc = "w.md"
    mkdir(base, cat)
    put_doc(base, cat, doc, "block")
    path = f"{cat}/{doc}"
    master, slave = pty.openpty()
    proc = subprocess.Popen(
        [tb, "-u", base, "-t", "120", "shell"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        close_fds=True,
        start_new_session=True,
    )
    os.close(slave)
    buf = bytearray()
    try:
        _pty_drain(master, buf, 1.5)
        os.write(master, f"wait {path}\n".encode())
        time.sleep(1.0)
        # Terminal Ctrl-C hits the foreground process group; parent shell ignores SIGINT
        # during `wait`, so we must signal the group — not only proc.pid.
        os.killpg(os.getpgid(proc.pid), signal.SIGINT)
        seen = False
        for _ in range(80):
            _pty_drain(master, buf, 0.2)
            if b"interrupted" in buf.lower():
                seen = True
                break
        assert seen, buf.decode("utf-8", errors="replace")
        time.sleep(0.3)
        os.write(master, b"ls\n")
        _pty_drain(master, buf, 4.0)
        os.write(master, b"exit\n")
        _pty_drain(master, buf, 4.0)
        proc.wait(timeout=60)
        assert proc.returncode == 0, buf.decode("utf-8", errors="replace")
        text = buf.decode("utf-8", errors="replace").lower()
        assert "interrupted by sigint" not in text
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=10)
        try:
            os.close(master)
        except OSError:
            pass
        _rpc("delete_directory", {"path": cat, "recursive": True})
