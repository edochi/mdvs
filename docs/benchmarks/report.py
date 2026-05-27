# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Render a benchmark JSON results file (produced by run.py) into a
Markdown report.

Usage:
    uv run docs/benchmarks/report.py docs/benchmarks/results/example_kb.json
    uv run docs/benchmarks/report.py docs/benchmarks/results/*.json -o docs/benchmarks/report.md

If multiple JSON files are passed, each becomes one corpus section in
a single combined report. Stable section order matches input file order.
"""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Constants — narrative sections kept at the top so the report contents are
# easy to inspect and tune without diving through formatting logic.
# ---------------------------------------------------------------------------

METHODOLOGY = """
## Methodology

For each (tool, corpus, query) combination the runner records:

- **Warm/steady-state search latency** — `/usr/bin/time -l` wall time, median of N iterations after one warm-up invocation
- **Peak resident set size** — maximum RSS observed during the query
- **CPU%** — derived from `(user + sys) / wall × 100`; indicates whether wall time was CPU-bound or I/O-bound
- **Index build time** — single timed run from clean state (`rm -rf .mdvs && mdvs build --force` for mdvs; `qmd collection add` + `qmd embed -f` for QMD)
- **Index size on disk** — `du -sk` after build
- **Output token count** — `tiktoken` cl100k_base over the result snippets, for `--limit 10`
- **Tool footprint on disk** — binary + cached embedding/reranker models

Cold-start latency and page-fault counts are deliberately excluded; see [TODO-0166](../spec/todos/TODO-0166.md) for the rationale.

Each search runs three iterations preceded by one warm-up invocation, so the reported wall time and RSS reflect steady-state behaviour.
""".strip()

FAIR_COMPARISON = """
## Reading the numbers fairly

The two tools have meaningfully different feature sets. Any conclusions drawn from these numbers should respect the following:

- **QMD `query` runs LLM reranking and query expansion** on top of BM25 + vector. mdvs's hybrid mode uses RRF only — no LLM in the loop. The reranking step changes both latency and quality; comparing wall times alone understates QMD's quality work
- **QMD does AST-aware chunking** for source code (TypeScript, JavaScript, Python, Go, Rust); mdvs uses prose chunking via `text-splitter`'s `MarkdownSplitter`. On code-heavy corpora the chunking strategies will produce different recall/precision profiles, independent of search engine speed
- **mdvs has `--where` SQL filtering and frontmatter validation**; QMD has neither. These are feature presence, not performance, and don't appear in the metric tables
- **Embedding models differ in size and quality.** mdvs uses Model2Vec `potion-base-8M` (~30 MB static distillation); QMD uses `embeddinggemma-300M-Q8_0` (~300 MB GGUF). Smaller model → less memory and faster load, but a different quality ceiling
- **Default chunking and limits differ.** mdvs uses 1024-char chunks; QMD's default chunking produces roughly one chunk per file on this corpus. Result count and token count comparisons should be read with this in mind

This benchmark measures **latency, footprint, and setup cost** under each tool's defaults. It does not measure ranking quality — that would require a labelled query set and is out of scope.
""".strip()

# ---------------------------------------------------------------------------
# Formatting helpers
# ---------------------------------------------------------------------------

def fmt_seconds(seconds: float) -> str:
    if seconds < 1.0:
        return f"{seconds * 1000:.0f} ms"
    return f"{seconds:.2f} s"

def fmt_bytes(b: int | None) -> str:
    if b is None:
        return "—"
    if b < 1024:
        return f"{b} B"
    kib = b / 1024
    if kib < 1024:
        return f"{kib:.1f} KB"
    mib = kib / 1024
    if mib < 1024:
        return f"{mib:.1f} MB" if mib < 100 else f"{mib:.0f} MB"
    gib = mib / 1024
    return f"{gib:.2f} GB"

def fmt_int(x: int | None) -> str:
    return f"{x:,}" if x is not None else "—"

def fmt_pct(p: float) -> str:
    return f"{p:.0f}%"

# ---------------------------------------------------------------------------
# Metric aggregation
# ---------------------------------------------------------------------------

def median_wall(iterations: list[dict]) -> float:
    return statistics.median(it["wall_s"] for it in iterations)

def max_rss(iterations: list[dict]) -> int:
    return max(it["peak_rss_bytes"] for it in iterations)

def median_cpu(iterations: list[dict]) -> float:
    return statistics.median(it["cpu_pct"] for it in iterations)

# ---------------------------------------------------------------------------
# Section renderers
# ---------------------------------------------------------------------------

def render_header(reports: list[dict]) -> str:
    now = datetime.now().strftime("%Y-%m-%d %H:%M %Z").strip()
    corpora = ", ".join(f"`{r['corpus_name']}`" for r in reports)
    versions = []
    if reports:
        tools = reports[0].get("tools", {})
        if "mdvs" in tools:
            versions.append(f"mdvs {tools['mdvs']['version'].removeprefix('mdvs ')}")
        if "qmd" in tools:
            versions.append(f"QMD {tools['qmd']['version'].removeprefix('qmd ')}")
    versions_line = " · ".join(versions) if versions else ""
    return f"""# mdvs vs QMD — benchmark report

_Generated {now}_
_Corpora: {corpora}_
_{versions_line}_

This report characterises how mdvs and QMD compare on warm/steady-state search latency, peak memory, build time, and output footprint. See [TODO-0166](../spec/todos/TODO-0166.md) for the framing and decisions behind what's measured (and what's deliberately not).
""".rstrip()

def render_environment(report: dict) -> str:
    m = report.get("machine", {})
    lines = ["## Test environment", "", "| | |", "|---|---|"]
    lines.append(f"| OS | `{m.get('os', '?')}` |")
    lines.append(f"| CPU arch | `{m.get('cpu', '?')}` |")
    lines.append(f"| Python | `{m.get('python', '?')}` |")
    for tool_key, tool in report.get("tools", {}).items():
        lines.append(f"| {tool_key} version | `{tool.get('version', '?')}` |")
    lines.append(f"| Iterations per query | {report.get('iterations', '?')} (+ 1 warm-up) |")
    lines.append(f"| --limit | {report.get('limit', '?')} |")
    return "\n".join(lines)

def render_corpus_section(report: dict) -> str:
    out: list[str] = []
    name = report["corpus_name"]
    files = report["corpus_files"]
    out.append(f"## Corpus: `{name}` ({files} files)")
    out.append("")
    out.append(_render_setup_table(report))
    out.append("")
    out.append(_render_queries_table(report))
    out.append("")
    out.append(_render_latency_table(report))
    out.append("")
    out.append(_render_token_table(report))
    notes = _render_notes(report)
    if notes:
        out.append("")
        out.append(notes)
    return "\n".join(out)

def _render_setup_table(report: dict) -> str:
    tools = report.get("tools", {})
    mdvs = tools.get("mdvs")
    qmd = tools.get("qmd")

    def row(label: str, mdvs_val: str, qmd_val: str) -> str:
        return f"| {label} | {mdvs_val} | {qmd_val} |"

    def setup_field(t: dict | None, field: str, default: str = "—") -> str:
        if t is None or t.get("setup") is None:
            return default
        return field if False else fmt_seconds(t["setup"]["wall_s"])

    lines = [
        "### Setup (one-time build cost)",
        "",
        "| | mdvs `build --force` | QMD `embed -f` |",
        "|---|---|---|",
    ]
    lines.append(row(
        "Wall time",
        fmt_seconds(mdvs["setup"]["wall_s"]) if mdvs and mdvs.get("setup") else "—",
        fmt_seconds(qmd["setup"]["wall_s"]) if qmd and qmd.get("setup") else "—",
    ))
    lines.append(row(
        "Peak RSS",
        fmt_bytes(mdvs["setup"]["peak_rss_bytes"]) if mdvs and mdvs.get("setup") else "—",
        fmt_bytes(qmd["setup"]["peak_rss_bytes"]) if qmd and qmd.get("setup") else "—",
    ))
    lines.append(row(
        "Index on disk",
        fmt_bytes(mdvs["index_size_bytes"]) if mdvs else "—",
        fmt_bytes(qmd["index_size_bytes"]) if qmd else "—",
    ))
    lines.append(row(
        "Embedding/reranker models on disk",
        fmt_bytes(mdvs["model_size_bytes"]) if mdvs else "—",
        fmt_bytes(qmd["model_size_bytes"]) if qmd else "—",
    ))
    return "\n".join(lines)

def _render_queries_table(report: dict) -> str:
    """Show the queries themselves so readers know what was searched."""
    tools = report.get("tools", {})
    # Use mdvs queries as the canonical list (it runs all five; QMD skips --where)
    src = tools.get("mdvs") or next(iter(tools.values()), None)
    if not src:
        return ""
    lines = [
        "### Queries",
        "",
        "| Kind | Query | mdvs mode | `--where` clause |",
        "|---|---|---|---|",
    ]
    for q in src.get("queries", []):
        where = f"`{q['where_clause']}`" if q.get("where_clause") else "—"
        lines.append(f"| `{q['kind']}` | _\"{q['query']}\"_ | `{q['mode']}` | {where} |")
    return "\n".join(lines)

def _render_latency_table(report: dict) -> str:
    tools = report.get("tools", {})
    mdvs = tools.get("mdvs")
    qmd = tools.get("qmd")

    kinds_in_order = [q["kind"] for q in (mdvs or qmd or {}).get("queries", [])]
    mdvs_by_kind = {q["kind"]: q for q in (mdvs or {}).get("queries", [])}
    qmd_by_kind = {q["kind"]: q for q in (qmd or {}).get("queries", [])}

    lines = [
        "### Search latency (warm, median of N)",
        "",
        "| Kind | mdvs wall | mdvs RSS | mdvs CPU% | QMD mode | QMD wall | QMD RSS | QMD CPU% |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for kind in kinds_in_order:
        mq = mdvs_by_kind.get(kind)
        qq = qmd_by_kind.get(kind)
        row = [f"`{kind}`"]
        if mq:
            row.extend([
                fmt_seconds(median_wall(mq["iterations"])),
                fmt_bytes(max_rss(mq["iterations"])),
                fmt_pct(median_cpu(mq["iterations"])),
            ])
        else:
            row.extend(["—", "—", "—"])
        if qq:
            row.extend([
                f"`{qq['mode']}`",
                fmt_seconds(median_wall(qq["iterations"])),
                fmt_bytes(max_rss(qq["iterations"])),
                fmt_pct(median_cpu(qq["iterations"])),
            ])
        else:
            row.extend(["—", "—", "—", "—"])
        lines.append("| " + " | ".join(row) + " |")
    return "\n".join(lines)

def _render_token_table(report: dict) -> str:
    tools = report.get("tools", {})
    mdvs = tools.get("mdvs")
    qmd = tools.get("qmd")
    kinds_in_order = [q["kind"] for q in (mdvs or qmd or {}).get("queries", [])]
    mdvs_by_kind = {q["kind"]: q for q in (mdvs or {}).get("queries", [])}
    qmd_by_kind = {q["kind"]: q for q in (qmd or {}).get("queries", [])}

    lines = [
        "### Output token count (snippets for `--limit 10`, `tiktoken` `cl100k_base`)",
        "",
        "Token count matters when results are piped into a downstream LLM — fewer tokens = less context spent.",
        "",
        "| Kind | mdvs result count | mdvs tokens | QMD result count | QMD tokens |",
        "|---|---|---|---|---|",
    ]
    for kind in kinds_in_order:
        mq = mdvs_by_kind.get(kind)
        qq = qmd_by_kind.get(kind)
        lines.append(
            f"| `{kind}` | {fmt_int(mq['result_count']) if mq else '—'} | {fmt_int(mq['output_token_count']) if mq else '—'}"
            f" | {fmt_int(qq['result_count']) if qq else '—'} | {fmt_int(qq['output_token_count']) if qq else '—'} |"
        )
    return "\n".join(lines)

def _render_notes(report: dict) -> str:
    lines = []
    for tool_key, tool in report.get("tools", {}).items():
        notes = tool.get("notes") or []
        for note in notes:
            lines.append(f"- _{tool_key}_: {note}")
    if not lines:
        return ""
    return "### Notes\n\n" + "\n".join(lines)

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[1])
    parser.add_argument("inputs", nargs="+", type=Path,
                        help="One or more JSON results files (each becomes a corpus section)")
    parser.add_argument("-o", "--output", type=Path,
                        help="Write Markdown to this path (default: stdout)")
    args = parser.parse_args()

    reports = []
    for p in args.inputs:
        if not p.exists():
            sys.exit(f"results file not found: {p}")
        reports.append(json.loads(p.read_text()))

    parts: list[str] = []
    parts.append(render_header(reports))
    parts.append("")
    parts.append(METHODOLOGY)
    parts.append("")
    parts.append(FAIR_COMPARISON)
    parts.append("")
    parts.append(render_environment(reports[0]))
    parts.append("")
    for r in reports:
        parts.append(render_corpus_section(r))
        parts.append("")

    md = "\n".join(parts).rstrip() + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(md)
        sys.stderr.write(f"wrote {args.output}\n")
    else:
        sys.stdout.write(md)

if __name__ == "__main__":
    main()
