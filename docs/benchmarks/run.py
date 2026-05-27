# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "rich>=13",
#   "tiktoken>=0.8",
# ]
# ///
"""
Benchmark runner for mdvs vs QMD (TODO-0166).

Measures warm/steady-state search latency, peak RSS, CPU%, build time,
index size, output token count, and tool footprint. Runs both tools
against a single corpus and writes a JSON results file.

Usage:
    uv run docs/benchmarks/run.py --corpus example_kb --output docs/benchmarks/results/example_kb.json

Methodology decisions (see docs/spec/todos/TODO-0166.md):
- Warm latency only (no cold-start)
- Page faults captured but not surfaced in the rendered report
- Each query runs N iterations; report uses median
- QMD `query` exit code 134 on macOS Metal teardown is tolerated; stdout is captured before the crash
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path

from rich.console import Console
from rich.table import Table

console = Console()

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parents[2]
MDVS_BIN = REPO_ROOT / "target" / "release" / "mdvs"
TIME_BIN = "/usr/bin/time"
DEFAULT_ITERATIONS = 3
DEFAULT_LIMIT = 10

# Five queries per the TODO. Each entry: (kind, query, mdvs_mode, where_clause).
# Per-corpus query sets so different corpora can use queries that fit their
# domain. Selection via --query-set on the CLI.
QUERY_SETS: dict[str, list[tuple[str, str, str, str | None]]] = {
    "example_kb": [
        ("broad_semantic",   "calibration baseline",                  "semantic",  None),
        ("narrow_semantic",  "wavelet denoising replication",         "semantic",  None),
        ("exact_phrase",     "SPR-A1",                                "fulltext",  None),
        ("metadata_filtered","calibration",                           "hybrid",    "status = 'completed'"),
        ("vague_multiword",  "what went wrong with the spectrometer", "hybrid",    None),
    ],
    "kubernetes": [
        ("broad_semantic",   "deploying applications to kubernetes",          "semantic",  None),
        ("narrow_semantic",  "rolling update strategy",                       "semantic",  None),
        ("exact_phrase",     "kubectl apply",                                 "fulltext",  None),
        ("metadata_filtered","minikube",                                      "hybrid",    "content_type = 'tutorial'"),
        ("vague_multiword",  "how do I expose my service to the internet",   "hybrid",    None),
    ],
}

# ---------------------------------------------------------------------------
# Data shapes
# ---------------------------------------------------------------------------

@dataclass
class Measurement:
    wall_s: float
    user_s: float
    sys_s: float
    cpu_pct: float
    peak_rss_bytes: int
    page_faults: int  # diagnostic only; not rendered
    page_reclaims: int  # diagnostic only; not rendered
    exit_code: int
    stdout_bytes: int

@dataclass
class QueryResult:
    kind: str
    query: str
    mode: str
    where_clause: str | None
    iterations: list[Measurement] = field(default_factory=list)
    # mdvs only: supplementary timings with `--no-update --no-build` so the
    # report can separate "search engine speed" from "default config including
    # auto-update + auto-build overhead". Empty/None for QMD (no equivalent).
    iterations_engine_only: list[Measurement] | None = None
    output_token_count: int | None = None
    result_count: int | None = None

@dataclass
class ToolResults:
    name: str
    version: str
    setup: Measurement | None = None
    index_size_bytes: int | None = None
    model_size_bytes: int | None = None
    queries: list[QueryResult] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

@dataclass
class BenchmarkRun:
    corpus_name: str
    corpus_files: int
    iterations: int
    limit: int
    machine: dict
    started_at: str
    tools: dict[str, ToolResults] = field(default_factory=dict)

# ---------------------------------------------------------------------------
# /usr/bin/time -l parsing
# ---------------------------------------------------------------------------

# Strip ANSI escape sequences (QMD's stderr emits cursor-control codes that
# would otherwise break the time-line regex).
_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]")

_TIME_LINE = re.compile(
    r"(?P<real>[\d.]+)\s+real\s+(?P<user>[\d.]+)\s+user\s+(?P<sys>[\d.]+)\s+sys",
)

def _grab_int(text: str, label: str) -> int:
    m = re.search(rf"\s+(\d+)\s+{re.escape(label)}", text)
    return int(m.group(1)) if m else 0

def parse_bsd_time(stderr: str, exit_code: int, stdout_bytes: int) -> Measurement:
    """Parse /usr/bin/time -l output (BSD format) from the tail of stderr."""
    stderr = _ANSI_RE.sub("", stderr)
    m = _TIME_LINE.search(stderr)
    if not m:
        raise RuntimeError(f"could not parse time output:\n{stderr[-2000:]}")
    real = float(m.group("real"))
    user = float(m.group("user"))
    sysc = float(m.group("sys"))
    rss = _grab_int(stderr, "maximum resident set size")
    faults = _grab_int(stderr, "page faults")
    reclaims = _grab_int(stderr, "page reclaims")
    cpu_pct = ((user + sysc) / real * 100.0) if real > 0 else 0.0
    return Measurement(
        wall_s=real,
        user_s=user,
        sys_s=sysc,
        cpu_pct=cpu_pct,
        peak_rss_bytes=rss,
        page_faults=faults,
        page_reclaims=reclaims,
        exit_code=exit_code,
        stdout_bytes=stdout_bytes,
    )

# ---------------------------------------------------------------------------
# Subprocess runner
# ---------------------------------------------------------------------------

def run_measured(cmd: list[str], *, env: dict | None = None,
                 tolerate_exit: bool = False) -> tuple[Measurement, str]:
    """Run cmd under /usr/bin/time -l. Return (Measurement, stdout)."""
    full = [TIME_BIN, "-l", *cmd]
    proc = subprocess.run(
        full,
        capture_output=True,
        text=True,
        env={**os.environ, **(env or {})},
        check=False,
    )
    if proc.returncode != 0 and not tolerate_exit:
        sys.stderr.write(proc.stderr)
        raise RuntimeError(f"command failed: {' '.join(cmd)} (exit {proc.returncode})")
    measurement = parse_bsd_time(proc.stderr, proc.returncode, len(proc.stdout))
    return measurement, proc.stdout

# ---------------------------------------------------------------------------
# Tool helpers — mdvs
# ---------------------------------------------------------------------------

def mdvs_version() -> str:
    out = subprocess.run([str(MDVS_BIN), "--version"], capture_output=True, text=True, check=True)
    return out.stdout.strip()

def mdvs_setup(corpus: Path) -> Measurement:
    """Force-rebuild the mdvs index. Returns the measured build."""
    mdvs_dir = corpus / ".mdvs"
    if mdvs_dir.exists():
        shutil.rmtree(mdvs_dir)
    m, _ = run_measured([str(MDVS_BIN), "build", str(corpus), "--force"])
    return m

def mdvs_query(corpus: Path, query: str, mode: str, where: str | None,
               limit: int, engine_only: bool = False) -> tuple[Measurement, str]:
    cmd = [
        str(MDVS_BIN), "search", query, str(corpus),
        "--mode", mode,
        "--limit", str(limit),
        "--output", "json",
    ]
    if where:
        cmd.extend(["--where", where])
    if engine_only:
        # Skip the auto-update + auto-build orchestration that fires before
        # every search by default (~110 ms overhead per query on example_kb).
        # Measures the search engine itself, not the orchestration cost.
        cmd.extend(["--no-update", "--no-build"])
    return run_measured(cmd)

def mdvs_index_size(corpus: Path) -> int:
    lance = corpus / ".mdvs" / "index.lance"
    return _du_bytes(lance) if lance.exists() else 0

def mdvs_model_size() -> int:
    """Best-effort: model cache directory under HF_HOME or default."""
    candidates = [
        Path.home() / ".cache" / "huggingface" / "hub",
    ]
    total = 0
    for d in candidates:
        if d.exists():
            for root, _, files in os.walk(d):
                for f in files:
                    if "potion-base-8m" in (Path(root) / f).as_posix().lower():
                        total += (Path(root) / f).stat().st_size
    return total

# ---------------------------------------------------------------------------
# Tool helpers — qmd
# ---------------------------------------------------------------------------

QMD_COLLECTION = "mdvs_bench"  # ephemeral; removed at start

def qmd_path() -> str:
    bin_ = shutil.which("qmd") or (Path.home() / ".bun" / "bin" / "qmd").as_posix()
    if not Path(bin_).exists():
        sys.exit("qmd not found; install via `bun install -g @tobilu/qmd`")
    return bin_

def qmd_env() -> dict:
    return {"PATH": f"{Path.home()}/.bun/bin:{os.environ['PATH']}"}

def qmd_version() -> str:
    out = subprocess.run([qmd_path(), "--version"], capture_output=True, text=True,
                         env={**os.environ, **qmd_env()}, check=True)
    return out.stdout.strip()

_QMD_CONFLICT_NAME = re.compile(r"^\s*Name:\s+(\S+)", re.MULTILINE)

def qmd_collection_add_with_retry(corpus: Path) -> list[str]:
    """Add a fresh QMD collection for the corpus. QMD enforces one collection per
    (path, pattern); its `collection list` hides paths, but the `add` error
    message names any conflicting collection. Parse it, remove, retry.

    Returns the list of pre-existing collection names we removed."""
    qmd = qmd_path()
    env = {**os.environ, **qmd_env()}
    removed: list[str] = []
    # First attempt: also remove our canonical name in case it's lingering
    subprocess.run([qmd, "collection", "remove", QMD_COLLECTION],
                   capture_output=True, env=env, check=False)
    for _ in range(3):
        result = subprocess.run(
            [qmd, "collection", "add", str(corpus), "--name", QMD_COLLECTION],
            capture_output=True, text=True, env=env, check=False,
        )
        if result.returncode == 0:
            return removed
        # On failure, the conflicting collection's name appears in stderr or stdout
        # as `Name: <name>`. Parse, remove, retry.
        m = _QMD_CONFLICT_NAME.search(result.stderr + result.stdout)
        if not m:
            sys.stderr.write(result.stderr or result.stdout)
            raise RuntimeError("qmd collection add failed and no conflicting name found")
        conflict = m.group(1)
        subprocess.run([qmd, "collection", "remove", conflict],
                       capture_output=True, env=env, check=False)
        removed.append(conflict)
    raise RuntimeError("qmd collection add: too many retries")

def qmd_setup(corpus: Path) -> tuple[Measurement, list[str]]:
    """Clean any prior collections for this path, add fresh, embed."""
    removed = qmd_collection_add_with_retry(corpus)
    # -f forces re-embedding of every chunk. Without it, content-hashed chunks
    # would be skipped across remove/add cycles, making build-time meaningless.
    m, _ = run_measured([qmd_path(), "embed", "-f"], env=qmd_env())
    return m, removed

def qmd_cleanup() -> None:
    """Remove our ephemeral benchmark collection from QMD's global index.

    Best-effort: any error is swallowed (we don't want cleanup failures to
    obscure the benchmark output). Safe to call multiple times — the QMD
    remove command is idempotent on missing collections."""
    try:
        subprocess.run(
            [qmd_path(), "collection", "remove", QMD_COLLECTION],
            capture_output=True,
            env={**os.environ, **qmd_env()},
            check=False,
        )
    except Exception:
        pass

def qmd_query(query: str, mode: str, limit: int) -> tuple[Measurement, str]:
    """mode is one of 'search', 'vsearch', 'query'."""
    cmd = [qmd_path(), mode, "-n", str(limit), query, "-c", QMD_COLLECTION, "--json"]
    # qmd query crashes on exit (Metal teardown) — tolerate exit 134
    tolerate = mode == "query"
    return run_measured(cmd, env=qmd_env(), tolerate_exit=tolerate)

def qmd_index_size() -> int:
    p = Path.home() / ".cache" / "qmd" / "index.sqlite"
    return p.stat().st_size if p.exists() else 0

def qmd_model_size() -> int:
    models_dir = Path.home() / ".cache" / "qmd" / "models"
    return _du_bytes(models_dir) if models_dir.exists() else 0

# Map mdvs mode -> qmd subcommand for fair comparison
MDVS_TO_QMD_MODE = {
    "semantic": "vsearch",
    "fulltext": "search",
    "hybrid":   "query",
}

# ---------------------------------------------------------------------------
# Token counting
# ---------------------------------------------------------------------------

def count_tokens(text: str) -> int:
    import tiktoken
    enc = tiktoken.get_encoding("cl100k_base")
    return len(enc.encode(text))

def extract_mdvs_snippets(stdout: str) -> tuple[list[str], int]:
    """Return (snippet_texts, result_count) from an mdvs --output json result."""
    try:
        doc = json.loads(stdout)
    except json.JSONDecodeError:
        return [], 0
    hits = doc.get("hits", [])
    snippets = [item.get("chunk_text", "") for item in hits]
    return snippets, len(hits)

def extract_qmd_snippets(stdout: str) -> tuple[list[str], int]:
    """Return (snippet_texts, result_count) from a qmd --json result."""
    try:
        items = json.loads(stdout)
    except json.JSONDecodeError:
        return [], 0
    snippets = [it.get("snippet", "") for it in items]
    return snippets, len(items)

# ---------------------------------------------------------------------------
# Disk usage helper
# ---------------------------------------------------------------------------

def _du_bytes(path: Path) -> int:
    """Disk usage via du -sk (in 1024-byte blocks, posix)."""
    out = subprocess.run(["du", "-sk", str(path)], capture_output=True, text=True, check=True)
    return int(out.stdout.split()[0]) * 1024

# ---------------------------------------------------------------------------
# Main run loop
# ---------------------------------------------------------------------------

def run_benchmark(
    corpus: Path,
    iterations: int,
    limit: int,
    tools: list[str],
    queries: list[tuple[str, str, str, str | None]],
) -> BenchmarkRun:
    md = corpus / "*.md"
    file_count = len(list(corpus.rglob("*.md")))
    machine = {
        "os": platform.platform(),
        "python": sys.version.split()[0],
        "cpu": platform.machine(),
    }
    run = BenchmarkRun(
        corpus_name=corpus.name,
        corpus_files=file_count,
        iterations=iterations,
        limit=limit,
        machine=machine,
        started_at=time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    )

    # ---- mdvs ----
    if "mdvs" in tools:
        console.rule("[bold cyan]mdvs setup")
        mdvs = ToolResults(name="mdvs", version=mdvs_version())
        mdvs.setup = mdvs_setup(corpus)
        mdvs.index_size_bytes = mdvs_index_size(corpus)
        mdvs.model_size_bytes = mdvs_model_size()
        console.print(f"build: wall={mdvs.setup.wall_s:.2f}s rss={mdvs.setup.peak_rss_bytes/1e6:.1f}MB")
        console.print(f"index: {mdvs.index_size_bytes/1e6:.2f}MB  model: {mdvs.model_size_bytes/1e6:.1f}MB")

        console.rule("[bold cyan]mdvs queries")
        for kind, query, mode, where in queries:
            qr = QueryResult(kind=kind, query=query, mode=mode, where_clause=where)
            qr.iterations_engine_only = []

            # Warm up under each configuration so the timed iterations are
            # measuring steady state, not the first invocation
            mdvs_query(corpus, query, mode, where, limit, engine_only=False)
            mdvs_query(corpus, query, mode, where, limit, engine_only=True)

            last_stdout = ""
            for _ in range(iterations):
                m, stdout = mdvs_query(corpus, query, mode, where, limit, engine_only=False)
                qr.iterations.append(m)
                last_stdout = stdout
            for _ in range(iterations):
                m, _ = mdvs_query(corpus, query, mode, where, limit, engine_only=True)
                qr.iterations_engine_only.append(m)

            snippets, count = extract_mdvs_snippets(last_stdout)
            qr.output_token_count = count_tokens("\n".join(snippets))
            qr.result_count = count
            mdvs.queries.append(qr)
            wall_default = statistics.median(m.wall_s for m in qr.iterations)
            wall_engine = statistics.median(m.wall_s for m in qr.iterations_engine_only)
            console.print(
                f"  {kind:18}  {mode:9}"
                f"  default={wall_default * 1000:6.1f}ms"
                f"  engine={wall_engine * 1000:6.1f}ms"
                f"  results={count}  tokens={qr.output_token_count}"
            )
        run.tools["mdvs"] = mdvs

    # ---- qmd ----
    # The QMD index lives at ~/.cache/qmd/index.sqlite globally on the user's
    # machine. Wrap setup + queries in try/finally so we don't leave our
    # ephemeral `mdvs_bench` collection behind if the run crashes or the user
    # interrupts mid-benchmark.
    if "qmd" in tools:
        try:
            console.rule("[bold magenta]qmd setup")
            qmd = ToolResults(name="qmd", version=qmd_version())
            qmd.setup, removed = qmd_setup(corpus)
            if removed:
                qmd.notes.append(f"removed existing collections for this path: {removed}")
            qmd.index_size_bytes = qmd_index_size()
            qmd.model_size_bytes = qmd_model_size()
            qmd.notes.append(
                "QMD uses a global ~/.cache/qmd/index.sqlite; index_size_bytes includes any unrelated user collections"
            )
            console.print(f"embed: wall={qmd.setup.wall_s:.2f}s rss={qmd.setup.peak_rss_bytes/1e6:.1f}MB")
            console.print(f"index: {qmd.index_size_bytes/1e6:.2f}MB  models: {qmd.model_size_bytes/1e6:.1f}MB")

            console.rule("[bold magenta]qmd queries")
            for kind, query, mdvs_mode, where in queries:
                if where is not None:
                    qmd.notes.append(f"skipped '{kind}': qmd has no --where equivalent")
                    continue
                qmd_mode = MDVS_TO_QMD_MODE[mdvs_mode]
                qr = QueryResult(kind=kind, query=query, mode=qmd_mode, where_clause=None)
                qmd_query(query, qmd_mode, limit)  # warm-up
                last_stdout = ""
                for i in range(iterations):
                    m, stdout = qmd_query(query, qmd_mode, limit)
                    qr.iterations.append(m)
                    last_stdout = stdout
                snippets, count = extract_qmd_snippets(last_stdout)
                qr.output_token_count = count_tokens("\n".join(snippets))
                qr.result_count = count
                qmd.queries.append(qr)
                wall = statistics.median(m.wall_s for m in qr.iterations)
                console.print(f"  {kind:18}  {qmd_mode:9}  wall={wall*1000:7.1f}ms  results={count}  tokens={qr.output_token_count}")
            run.tools["qmd"] = qmd
        finally:
            qmd_cleanup()
            console.print("[dim]qmd: removed benchmark collection from global index[/dim]")

    return run

# ---------------------------------------------------------------------------
# Summary table
# ---------------------------------------------------------------------------

def render_summary(run: BenchmarkRun, queries: list[tuple[str, str, str, str | None]]) -> None:
    console.rule("[bold green]Summary (median across iterations)")
    table = Table(show_header=True)
    table.add_column("kind")
    table.add_column("mdvs mode")
    table.add_column("mdvs default")
    table.add_column("mdvs engine")
    table.add_column("qmd mode")
    table.add_column("qmd wall")

    mdvs = run.tools.get("mdvs")
    qmd = run.tools.get("qmd")
    mdvs_by_kind = {q.kind: q for q in (mdvs.queries if mdvs else [])}
    qmd_by_kind = {q.kind: q for q in (qmd.queries if qmd else [])}

    for kind, _, _, _ in queries:
        mq = mdvs_by_kind.get(kind)
        qq = qmd_by_kind.get(kind)
        row = [kind]
        if mq:
            default_wall = statistics.median(m.wall_s for m in mq.iterations) * 1000
            engine_wall = (
                statistics.median(m.wall_s for m in mq.iterations_engine_only) * 1000
                if mq.iterations_engine_only
                else None
            )
            row.extend([
                mq.mode,
                f"{default_wall:.1f}ms",
                f"{engine_wall:.1f}ms" if engine_wall is not None else "—",
            ])
        else:
            row.extend(["-", "-", "-"])
        if qq:
            wall = statistics.median(m.wall_s for m in qq.iterations) * 1000
            row.extend([qq.mode, f"{wall:.1f}ms"])
        else:
            row.extend(["-", "-"])
        table.add_row(*row)
    console.print(table)

# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[1] if __doc__ else "")
    parser.add_argument("--corpus", required=True, type=Path, help="Path to the markdown corpus")
    parser.add_argument("--output", required=True, type=Path, help="JSON results path")
    parser.add_argument("--iterations", type=int, default=DEFAULT_ITERATIONS)
    parser.add_argument("--limit", type=int, default=DEFAULT_LIMIT)
    parser.add_argument("--tools", default="mdvs,qmd",
                        help="Comma-separated subset of {mdvs,qmd}")
    parser.add_argument("--query-set", required=True,
                        choices=sorted(QUERY_SETS.keys()),
                        help="Which query set to use (defined in QUERY_SETS)")
    args = parser.parse_args()

    if not MDVS_BIN.exists():
        sys.exit(f"mdvs release binary not found at {MDVS_BIN} — run `cargo build --release` first")

    tools = [t.strip() for t in args.tools.split(",") if t.strip()]
    corpus = args.corpus.resolve()
    if not corpus.is_dir():
        sys.exit(f"corpus not found or not a directory: {corpus}")

    queries = QUERY_SETS[args.query_set]

    run = run_benchmark(corpus, args.iterations, args.limit, tools, queries)
    render_summary(run, queries)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(_serialize(run), indent=2))
    console.print(f"\n[bold green]results written to {args.output}")

def _serialize(run: BenchmarkRun) -> dict:
    """asdict, but convert nested ToolResults / Measurement properly."""
    out = asdict(run)
    return out

if __name__ == "__main__":
    main()
