#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.9"
# dependencies = [
#     "pexpect>=4.9",
# ]
# ///
"""Scripted asciinema demo for the mdvs README.

Drives a fresh, minimal bash inside asciinema's recorder so the demo is
fully reproducible from source. Re-run after rebuilding mdvs to refresh
``demo.cast``.

Prerequisites
-------------
- ``asciinema`` (3.x) on ``$PATH`` (``cargo install asciinema`` or ``brew
  install asciinema``).
- ``uv`` on ``$PATH`` (https://docs.astral.sh/uv/).
- ``cargo build --release -p mdvs`` (the demo invokes
  ``target/release/mdvs``).
- ``example_kb`` already built: ``./target/release/mdvs build example_kb``.

Run
---
From the repo root::

    uv run assets/demo.py

Render to GIF (optional, once ``agg`` is installed)::

    agg assets/demo.cast assets/demo.gif --theme monokai --font-size 16
"""

from __future__ import annotations

import os
import shutil
import tempfile
import time
from pathlib import Path

import pexpect

REPO = Path(__file__).resolve().parent.parent
CAST = Path(__file__).parent / "demo.cast"
PROMPT = r"\$ $"


def main() -> None:
    asciinema = shutil.which("asciinema")
    if asciinema is None:
        raise SystemExit(
            "asciinema not on PATH — install with "
            "`pip install -r assets/requirements.txt` (preferably inside a venv)."
        )

    binary = REPO / "target" / "release" / "mdvs"
    if not binary.exists():
        raise SystemExit(
            f"{binary} not found — run `cargo build --release -p mdvs` first."
        )

    with tempfile.NamedTemporaryFile(
        "w", prefix="mdvs-demo-rc-", suffix=".sh", delete=False
    ) as rc:
        rc.write("PS1='$ '\n")
        rc.write(f'export PATH="{binary.parent}:$PATH"\n')
        rcfile = rc.name

    env = os.environ.copy()
    # Silences macOS bash 3.2's "use zsh" advisory that prints at startup.
    env["BASH_SILENCE_DEPRECATION_WARNING"] = "1"

    try:
        child = pexpect.spawn(
            asciinema,
            args=[
                "rec",
                str(CAST),
                "--overwrite",
                "--window-size",
                "110x30",
                "--command",
                f"bash --noprofile --rcfile {rcfile}",
                "--quiet",
            ],
            encoding="utf-8",
            timeout=60,
            cwd=str(REPO),
            env=env,
        )
        child.expect(PROMPT)

        def run(cmd: str, after: float = 1.0, char_delay: float = 0.045) -> None:
            for c in cmd:
                child.send(c)
                time.sleep(char_delay)
            child.send("\r")
            child.expect(PROMPT)
            time.sleep(after)

        run("mdvs --version", after=3.0)
        run("cd example_kb", after=0.6)
        run("mdvs info", after=3.5)
        run("mdvs search 'experiment' --limit 1", after=3.5)
        run(
            "mdvs search 'experiment' --limit 1 "
            "--where \"date BETWEEN '2031-09-01' AND '2031-11-30'\"",
            after=4.0,
        )

        child.sendline("exit")
        child.expect(pexpect.EOF)
    finally:
        os.unlink(rcfile)


if __name__ == "__main__":
    main()
