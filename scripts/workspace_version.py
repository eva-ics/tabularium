#!/usr/bin/env python3
"""Bump or set [workspace.package] version in the workspace-root Cargo.toml (stdlib only)."""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO = ROOT / "Cargo.toml"

SEMVER_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
VERSION_LINE_RE = re.compile(r"^(\s*)version\s*=\s*\"([^\"]+)\"(.*)$")


def read_text() -> str:
    return CARGO.read_text(encoding="utf-8")


def write_text(text: str) -> None:
    CARGO.write_text(text, encoding="utf-8")


def workspace_package_version_span(text: str) -> tuple[int, int, str]:
    """Return (start, end, current_version) of the version assignment under [workspace.package]."""
    lines = text.splitlines(keepends=True)
    in_workspace_package = False
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[workspace.package]":
            in_workspace_package = True
            continue
        if in_workspace_package and stripped.startswith("[") and stripped.endswith("]"):
            break
        if not in_workspace_package:
            continue
        m = VERSION_LINE_RE.match(line.rstrip("\n"))
        if m:
            start = sum(len(lines[j]) for j in range(i))
            end = start + len(line)
            return start, end, m.group(2)
    sys.exit("no version = \"...\" under [workspace.package] in workspace Cargo.toml")


def replace_version(text: str, new_ver: str) -> str:
    start, end, _old = workspace_package_version_span(text)
    segment = text[start:end]
    line_m = VERSION_LINE_RE.match(segment.rstrip("\n"))
    if not line_m:
        sys.exit(f"cannot parse version line: {segment!r}")
    indent, _old_v, tail = line_m.groups()
    nl = "\n" if segment.endswith("\n") else ""
    new_line = f'{indent}version = "{new_ver}"{tail}{nl}'
    return text[:start] + new_line + text[end:]


def parse_semver(v: str) -> tuple[int, int, int]:
    m = SEMVER_RE.match(v.strip())
    if not m:
        sys.exit(f"expected X.Y.Z (digits only), got {v!r}")
    return int(m.group(1)), int(m.group(2)), int(m.group(3))


def bump_patch(v: str) -> str:
    major, minor, patch = parse_semver(v)
    return f"{major}.{minor}.{patch + 1}"


def run_cargo_check() -> None:
    subprocess.run(
        ["cargo", "check", "--workspace"],
        cwd=ROOT,
        check=True,
    )


def main(argv: list[str]) -> None:
    if len(argv) < 2:
        sys.exit("usage: workspace_version.py bump | workspace_version.py set X.Y.Z")

    cmd = argv[1]
    text = read_text()

    if cmd == "bump":
        _s, _e, cur = workspace_package_version_span(text)
        new_ver = bump_patch(cur)
        write_text(replace_version(text, new_ver))
        print(f"bumped workspace version {cur} -> {new_ver}")
        run_cargo_check()
        return

    if cmd == "set":
        if len(argv) != 3:
            sys.exit("usage: workspace_version.py set X.Y.Z")
        new_ver = argv[2]
        parse_semver(new_ver)
        _s, _e, cur = workspace_package_version_span(text)
        write_text(replace_version(text, new_ver))
        print(f"set workspace version {cur} -> {new_ver}")
        run_cargo_check()
        return

    sys.exit(f"unknown command {cmd!r}; use bump or set")


if __name__ == "__main__":
    main(sys.argv)
