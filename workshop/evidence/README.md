# Local startup evidence

Measured on the workshop machine on September 27, 2026 using installed Python 3.12.1 and Rust 1.98.1. No compiler, dependency, or tool was installed.

| Probe | Median | p95 |
| --- | ---: | ---: |
| Optimized Rust, standard library only | 12.1 ms | 16.8 ms |
| Python, standard library only | 39.0 ms | 55.6 ms |
| Python also importing sqlite3 | 54.3 ms | 76.6 ms |

The first two programs do the same tiny task: accept `status` and print `agentboard: ok`. Every invocation's output and exit status were checked. The third is context about an additional Python import, **not an equivalent Rust-versus-Python database comparison**. No variant opens a database.

Each variant received five unrecorded warmups and 50 measured invocations. The variants were interleaved in seeded random order. The running Python harness measures `subprocess.run` with `perf_counter_ns`, waiting for process exit and collecting stdout/stderr through pipes. There is no shell per invocation. The p95 is the nearest-rank 95th percentile.

These are **warm local Windows process baselines**, not cold-start results or predictions of Agentboard latency. They include process creation, runtime initialization, tiny argument/output work, shutdown, and pipe collection. They exclude realistic database operations, CLI frameworks, schema setup, concurrency and packaging. Background load, antivirus, hardware, caches, and installation choices can change them. Rust's result demonstrates lower baseline overhead here; it does not establish the latency of a Rust Agentboard implementation.

Run again from the repository root using `python workshop/evidence/measure_startup.py`. This overwrites `startup-results.json` with a fresh run. Rust compilation and its executable are kept in a temporary directory and cleaned up afterward. The JSON contains complete samples and environment metadata. The two Python sources and Rust source are included alongside it.
