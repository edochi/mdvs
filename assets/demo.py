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
DEMO_KB = Path(__file__).parent / "demo_kb"
PROMPT = r"\$ $"


def reset_demo_kb() -> None:
    """Remove generated state inside ``demo_kb`` so ``mdvs init`` runs clean."""
    for path in (
        DEMO_KB / "mdvs.toml",
        DEMO_KB / ".mdvs",
        DEMO_KB / "books" / "draft.md",
    ):
        if path.is_dir():
            shutil.rmtree(path)
        elif path.exists():
            path.unlink()


def main() -> None:
    asciinema = shutil.which("asciinema")
    if asciinema is None:
        raise SystemExit(
            "asciinema not on PATH — install it with "
            "`cargo install asciinema` or `brew install asciinema`."
        )

    binary = REPO / "target" / "release" / "mdvs"
    if not binary.exists():
        raise SystemExit(
            f"{binary} not found — run `cargo build --release -p mdvs` first."
        )

    reset_demo_kb()

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

        def comment(text: str, after: float = 1.2) -> None:
            for c in f"# {text}":
                child.send(c)
                time.sleep(0.025)
            child.send("\r")
            child.expect(PROMPT)
            time.sleep(after)

        run("cd assets/demo_kb", after=0.6)

        comment("The knowledge base — six markdown notes:", after=1.0)
        run("find . -name '*.md' | sort", after=2.5)

        comment("Each file has YAML frontmatter:", after=1.0)
        run("cat books/dune.md", after=3.0)

        comment("Scan, infer the typed schema, write mdvs.toml:", after=1.0)
        run("mdvs init", after=4.5)

        comment("Validate the files against the schema:", after=1.0)
        run("mdvs check", after=3.0)

        comment("Add a draft with a wrong-type field:", after=1.0)
        run(
            "printf -- '---\\ntitle: Draft\\nrating: TBD\\n---\\n' "
            "> books/draft.md",
            after=0.6,
        )
        run("cat books/draft.md", after=2.5)

        comment("check catches the violations:", after=1.0)
        run("mdvs check", after=4.5)

        comment("Remove the bad file:", after=1.0)
        run("rm books/draft.md", after=1.0)

        comment("Build the search index (local embeddings):", after=1.0)
        run("mdvs build", after=4.0)

        comment("Semantic search + SQL filter:", after=1.0)
        run(
            "mdvs search 'sci-fi' --limit 2 "
            "--where \"date_added > '2024-02-01'\"",
            after=5.0,
        )

        # Ctrl+D — bash exits on EOF. It still echoes "exit\r\n" because
        # that's hardcoded for interactive shells; the post-processing step
        # below strips that final event from the cast.
        child.sendcontrol("d")
        child.expect(pexpect.EOF)
    finally:
        os.unlink(rcfile)

    strip_trailing_exit(CAST)


def strip_trailing_exit(cast_path: Path) -> None:
    """Drop bash's trailing ``exit\\r\\n`` echo from the recorded cast."""
    lines = cast_path.read_text().splitlines(keepends=True)
    cleaned = [
        line
        for line in lines
        if not (line.startswith("[") and '"o", "exit\\r\\n"' in line)
    ]
    cast_path.write_text("".join(cleaned))


if __name__ == "__main__":
    main()
