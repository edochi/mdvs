# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Backend comparison runner: the same storage and search workload against each
backend, on the same data.

Exports a built mdvs index once (scripts/bench_backend_export.rs), runs each
backend script (scripts/bench_backend_<name>.rs) on it, and prints a
comparison. Queries come from QUERY_SETS in run.py.

Usage:
    uv run docs/benchmarks/backends.py --corpus <vault> --query-set kubernetes \
        [--backends lance,turso] [--reps 20] [--output results/backends-k8s.json]

The corpus must already be built with mdvs (`mdvs build`), with the model the
queries should be embedded with. Needs rust-script on PATH.
"""

from __future__ import annotations

import argparse
import ast
import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPTS = REPO / "scripts"


def query_sets() -> dict:
    """Read QUERY_SETS from run.py without importing it (it needs extra deps)."""
    tree = ast.parse((Path(__file__).parent / "run.py").read_text())
    for node in tree.body:
        if isinstance(node, ast.AnnAssign) and getattr(node.target, "id", None) == "QUERY_SETS":
            return ast.literal_eval(node.value)
    sys.exit("QUERY_SETS not found in run.py")


def to_queries(entries: list) -> list[dict]:
    """Turn run.py's (kind, text, mode, where) tuples into filter objects."""
    out = []
    for kind, text, mode, where in entries:
        flt = None
        if where:
            field, _, value = where.partition("=")
            flt = {"field": field.strip(), "value": value.strip().strip("'")}
        out.append({"kind": kind, "text": text, "mode": mode, "filter": flt})
    return out


def rust_script(name: str, *args: str) -> str:
    cmd = ["rust-script", str(SCRIPTS / f"{name}.rs"), *args]
    print(f"$ {' '.join(cmd)}", file=sys.stderr)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr[-4000:])
        sys.exit(f"{name} failed with exit code {proc.returncode}")
    return proc.stdout


def overlap(a: list[str], b: list[str]) -> float:
    if not a and not b:
        return 1.0
    return len(set(a) & set(b)) / max(len(a), len(b))


def report(results: list[dict]) -> str:
    names = [r["backend"] for r in results]
    lines = [f"Rows: {results[0]['rows']}, warm runs per query: {results[0]['reps']}", ""]
    head = "| Metric | " + " | ".join(names) + " |"
    lines += [head, "|---" * (len(names) + 1) + "|"]

    def row(label: str, fmt) -> None:
        lines.append(f"| {label} | " + " | ".join(fmt(r) for r in results) + " |")

    row("Insert", lambda r: f"{r['insert_ms']:,.0f} ms")
    row("Index build", lambda r: f"{r['index_ms']:,.0f} ms")
    row("Open", lambda r: f"{r['open_ms']:,.1f} ms")
    row("Incremental (50 files)", lambda r: f"{r['incremental_ms']:,.0f} ms ({r['incremental_rows']} rows)")
    row("On disk", lambda r: f"{r['disk_bytes'] / 1e6:,.1f} MB")
    row("Executable", lambda r: f"{r['exe_bytes'] / 1e6:,.1f} MB")

    lines += ["", "Query latency, p50 / p95:", ""]
    lines += [head.replace("Metric", "Query"), "|---" * (len(names) + 1) + "|"]
    for i, q in enumerate(results[0]["queries"]):
        label = f"{q['kind']} ({q['mode']})"
        lines.append(
            f"| {label} | "
            + " | ".join(f"{r['queries'][i]['p50_ms']:.2f} / {r['queries'][i]['p95_ms']:.2f} ms" for r in results)
            + " |"
        )

    if len(results) > 1:
        base = results[0]
        lines += ["", f"Top-10 overlap with {base['backend']}:", ""]
        lines += ["| Query | " + " | ".join(names[1:]) + " |", "|---" * len(names) + "|"]
        for i, q in enumerate(base["queries"]):
            lines.append(
                f"| {q['kind']} ({q['mode']}) | "
                + " | ".join(f"{overlap(q['top_ids'], r['queries'][i]['top_ids']):.0%}" for r in results[1:])
                + " |"
            )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--corpus", required=True, type=Path, help="A vault already built with mdvs")
    parser.add_argument("--query-set", required=True, help="Key of QUERY_SETS in run.py")
    parser.add_argument("--backends", default="lance,turso", help="Comma-separated backend script names")
    parser.add_argument("--reps", type=int, default=20)
    parser.add_argument("--work", type=Path, default=Path(tempfile.gettempdir()) / "mdvs-backend-bench")
    parser.add_argument("--re-export", action="store_true", help="Re-export the dataset even if cached")
    parser.add_argument("--output", type=Path, help="Write all results as JSON")
    args = parser.parse_args()

    sets = query_sets()
    if args.query_set not in sets:
        sys.exit(f"unknown query set {args.query_set!r}; have {sorted(sets)}")

    dataset = args.work / "dataset" / args.query_set
    if args.re_export or not (dataset / "chunks.jsonl").exists():
        dataset.mkdir(parents=True, exist_ok=True)
        queries = dataset / "queries.json"
        queries.write_text(json.dumps(to_queries(sets[args.query_set]), indent=1))
        rust_script("bench_backend_export", str(args.corpus.expanduser()), str(queries), str(dataset))

    results = []
    for name in args.backends.split(","):
        work = args.work / "stores" / args.query_set
        work.mkdir(parents=True, exist_ok=True)
        results.append(json.loads(rust_script(f"bench_backend_{name}", str(dataset), str(work), str(args.reps))))

    print(report(results))
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps({"corpus": str(args.corpus), "query_set": args.query_set, "results": results}, indent=2))


if __name__ == "__main__":
    main()
