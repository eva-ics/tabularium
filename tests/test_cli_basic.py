import subprocess

from tests.helpers import _base, _tb_bin


def test_cli_smoke():
    tb = _tb_bin()
    base = _base()
    r = subprocess.run(
        [tb, "-u", base, "ls"],
        capture_output=True,
        text=True,
        timeout=15,
        check=False,
    )
    assert r.returncode == 0, r.stderr
