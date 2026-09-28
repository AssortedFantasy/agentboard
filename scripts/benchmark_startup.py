"""Measure warm native CLI invocations, including real SQLite and stdout work.

Usage: python scripts/benchmark_startup.py target/release/agentboard[.exe]
Only Python's standard library is needed; this is a developer measurement tool.
"""
import json
import platform
import random
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def main():
    binary = str(Path(sys.argv[1]).resolve())
    samples = {"version": [], "post_list": [], "query": []}
    with tempfile.TemporaryDirectory(prefix="agentboard-benchmark-") as directory:
        board = str(Path(directory) / "board.db")
        prefix = [binary, board, "benchmark"]
        subprocess.run(prefix + ["post", "create", "/", "--title", "Benchmark", "--body", "Small representative post."], check=True, capture_output=True)
        commands = {
            "version": [binary, "--version"],
            "post_list": prefix + ["post", "list", "--json"],
            "query": prefix + ["query", "SELECT id FROM posts", "--render", "post", "--json"],
        }
        for command in commands.values():
            for _ in range(5):
                subprocess.run(command, check=True, capture_output=True)
        trials = list(commands) * 50
        random.Random(42).shuffle(trials)
        for name in trials:
            started = time.perf_counter_ns()
            result = subprocess.run(commands[name], check=True, capture_output=True)
            samples[name].append((time.perf_counter_ns() - started) / 1_000_000)
            if name != "version":
                assert len(json.loads(result.stdout)["items"]) == 1
        sqlite = json.loads(subprocess.run(prefix + ["query", "SELECT sqlite_version() AS version", "--json"], check=True, capture_output=True).stdout)["items"][0]["version"]
    report = {
        "platform": platform.platform(),
        "binary_bytes": Path(binary).stat().st_size,
        "sqlite_version": sqlite,
        "method": "50 randomized interleaved warm subprocess trials per command, 5 warmups each, shell=False, stdout/stderr captured, one-post real SQLite board; list/query include telemetry and receipts",
        "results": {name: {"median_ms": round(statistics.median(values), 3), "p95_ms": round(sorted(values)[47], 3), "samples_ms": [round(value, 3) for value in values]} for name, values in samples.items()},
    }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
