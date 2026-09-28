"""Run with Python; requires an already installed rustc. Installs nothing.

Build output goes in a temporary directory. Results are written beside this file.
Each sample measures subprocess.run through process exit and pipe collection.
"""
import datetime
import json
import math
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time


def main():
    here = Path(__file__).resolve().parent
    rustc = shutil.which("rustc")
    if not rustc:
        raise SystemExit("rustc is not installed; no Rust measurement is available")
    with tempfile.TemporaryDirectory(prefix="agentboard-startup-") as build:
        binary = Path(build) / ("toy_cli.exe" if os.name == "nt" else "toy_cli")
        subprocess.run([rustc, "-O", str(here / "toy_cli.rs"), "-o", str(binary)], check=True)
        commands = {
            "rust_stdlib": [str(binary), "status"],
            "python_stdlib": [sys.executable, str(here / "toy_cli.py"), "status"],
            "python_import_sqlite3": [sys.executable, str(here / "toy_cli_sqlite.py"), "status"],
        }
        def run(command):
            start = time.perf_counter_ns()
            result = subprocess.run(command, capture_output=True, check=True)
            elapsed = (time.perf_counter_ns() - start) / 1_000_000
            assert result.stdout.replace(b"\r\n", b"\n") == b"agentboard: ok\n", result
            assert not result.stderr, result
            return elapsed
        for command in commands.values():
            for _ in range(5):
                run(command)
        samples = {name: [] for name in commands}
        rng = random.Random(20260927)
        for _ in range(50):
            order = list(commands)
            rng.shuffle(order)
            for name in order:
                samples[name].append(run(commands[name]))
        report = {
            "measured_at": datetime.datetime.now().astimezone().isoformat(),
            "platform": platform.platform(),
            "processor": platform.processor(),
            "python": sys.version,
            "python_executable": sys.executable,
            "rustc": subprocess.check_output([rustc, "--version"], text=True).strip(),
            "rust_optimization": "rustc -O (default target)",
            "method": "5 warmups per variant, then 50 interleaved samples each in seeded randomized order; subprocess.run with shell=False, stdout/stderr pipes, process exit awaited; perf_counter_ns elapsed",
            "p95_definition": "nearest rank: sorted samples[ceil(0.95*n)-1]",
            "limitations": [
                "Local Windows warm process baseline; not cold launch or Agentboard performance.",
                "Rust and base Python only parse a status argument and print the same line.",
                "sqlite3 variant imports Python sqlite3 only; neither opens a database. Rust has no comparable SQLite dependency, so this variant is asymmetric context only.",
                "Includes OS process creation, runtime initialization, tiny task, shutdown and pipe collection; benchmark Python runner is already running.",
                "No third-party CLI parser, packaging, schema setup, database operation, concurrency, or realistic payload measured.",
                "Background activity, cache warmth, antivirus, installation and hardware affect these measurements; no speed guarantee.",
            ],
            "results": {
                name: {
                    "n": len(values),
                    "median_ms": statistics.median(values),
                    "p95_ms": sorted(values)[math.ceil(0.95 * len(values)) - 1],
                    "min_ms": min(values),
                    "max_ms": max(values),
                    "samples_ms": values,
                }
                for name, values in samples.items()
            },
        }
        (here / "startup-results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({k: {m: v for m, v in d.items() if m != "samples_ms"} for k, d in report["results"].items()}, indent=2))


if __name__ == "__main__":
    main()
