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
- ``jq`` on ``$PATH`` (final demo beat pipes ``--output json`` through it).
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
        DEMO_KB / "wiki" / "concepts" / "llm-tooling.md",
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
    if shutil.which("jq") is None:
        raise SystemExit("jq not on PATH — install it (brew install jq).")
    if shutil.which("bat") is None:
        raise SystemExit("bat not on PATH — install it (brew install bat).")

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
        # Recording-only alias: viewers see `cat file.md` but get bat's
        # syntax-highlighted output. The rcfile is temp and removed
        # below, so this never leaves the recording session.
        rc.write(
            "alias cat='bat --color=always --paging=never --style=plain "
            "--theme=TwoDark'\n"
        )
        # `expand_aliases` lets non-interactive bash (which doesn't expand
        # aliases by default) honour the alias even when sub-acts in this
        # script run via pexpect. Without it, the alias only fires for
        # top-level prompt commands — which is what we want, but flip it on
        # for safety against shell version quirks.
        rc.write("shopt -s expand_aliases\n")
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

        def run(cmd: str, after: float = 0.8, char_delay: float = 0.035) -> None:
            for c in cmd:
                child.send(c)
                time.sleep(char_delay)
            # Hold for a beat after the command is fully typed — gives the
            # viewer time to read the command before the output replaces it.
            time.sleep(1.0)
            child.send("\r")
            child.expect(PROMPT)
            time.sleep(after)

        def comment(text: str, after: float = 0.4) -> None:
            # Comments type at a readable cadence — slow enough to be
            # readable as they appear, faster than commands since they're
            # scaffolding rather than content.
            for c in f"# {text}":
                child.send(c)
                time.sleep(0.025)
            child.send("\r")
            child.expect(PROMPT)
            time.sleep(after)

        def clear(after: float = 0.3) -> None:
            """Send Ctrl-L so each section starts on a clean terminal."""
            child.sendcontrol("l")
            child.expect(PROMPT)
            time.sleep(after)

        run("cd assets/demo_kb", after=0.4)

        # Act 1 — show one file's frontmatter (the structure is implicit in the path).
        # `cat` is aliased to `bat` in the rcfile so files render with syntax
        # highlighting; viewers see the familiar `cat` command.
        comment("A small AI-curated knowledge base. Each entry is typed:")
        run("cat wiki/concepts/vector-databases.md", after=7.0)

        clear()

        # Act 2 — schema inference.
        comment("mdvs infers the schema from the existing files:")
        run("mdvs init", after=3.0)

        # Act 2b — surface the categorical inference; it sets up the violation.
        comment(
            "It even spotted that confidence is a closed category — "
            "three valid values:"
        )
        run("grep --color=always -B 1 -A 1 'categories' mdvs.toml", after=5.0)

        clear()

        # Act 3 — agent writes a new entry with a hallucinated category value.
        comment(
            "An LLM agent drafts a new entry — but invents 'TBD', "
            "a value not in the schema:"
        )
        run(
            "printf -- '---\\ntitle: LLM tooling\\ncategory: model\\nconfidence: TBD\\n"
            "last_reviewed: 2026-06-01\\ntags: [llm, tools]\\n---\\n' "
            "> wiki/concepts/llm-tooling.md",
            after=0.3,
        )
        run("cat wiki/concepts/llm-tooling.md", after=5.0)

        comment(
            "check fires — TBD isn't one of the categories mdvs inferred "
            "(high / low / medium):"
        )
        run("mdvs check", after=9.0)

        clear()

        # Act 4 — show the schema-widening resolution. The intuitive fix is
        # to change the value in the file; the mdvs-distinctive fix is to
        # widen the schema. We narrate option 1 and execute option 2.
        comment(
            "Two ways to fix this. The agent could change the value to one "
            "of the valid categories."
        )
        comment(
            "Or — if TBD is actually a legitimate state — widen the schema "
            "by adding it to mdvs.toml:"
        )
        run(
            "sed -i.bak 's/\\[\"high\", \"low\", \"medium\"\\]/"
            "[\"TBD\", \"high\", \"low\", \"medium\"]/' "
            "mdvs.toml && rm mdvs.toml.bak",
            after=0.3,
        )
        run("grep --color=always -B 1 -A 1 'categories' mdvs.toml", after=5.0)

        comment("Now check passes — TBD is part of the schema:")
        run("mdvs check", after=5.0)

        clear()

        # Act 5 — build the search index.
        comment("Build the search index (local embeddings):")
        run("mdvs build", after=5.0)

        clear()

        # Act 6 — typed-frontmatter semantic search.
        comment("Semantic search with a typed --where filter:")
        run(
            "mdvs search 'how do agents query a vector store' "
            "--where \"category = 'infra' AND confidence != 'low'\" --limit 3",
            after=9.0,
        )

        clear()

        # Act 7 — agent-callable surface. This is the headline.
        comment("And the agent-callable surface — JSON out, jq in:")
        run(
            "mdvs search 'retrieval pipeline' --limit 4 --output json | "
            "jq '.hits[] | {file: .filename, score: .score}'",
            after=9.0,
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
