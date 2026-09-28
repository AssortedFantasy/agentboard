# V1 validation — 28 September 2026

## Windows

Windows 11, Rust/Cargo 1.98.1, bundled SQLite 3.53.2:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo test --locked`: **84 passed**, none failed.
- `cargo build --locked --release`: passed.
- Web appearance inspected in Chrome: plain content, ordinary links, restrained boxes; authored HTML escaped.

The [requirement audit](AUDIT.md) connects each original scope item and subsequent decision with implementation and tests. Integration regressions cover concurrent first initialization, atomic competing claims, dependency rollback, live waits, automatic notification routing, archive inheritance, output limits, broken pipes, SQL guards, command logs, and real HTTP requests.

## Startup measurements

The native release executable was measured with 50 interleaved warm subprocess trials per command, after five warmups each. The database contained one post. Output was captured and checked; database commands include observation receipts and command logging with FULL synchronous commits.

| Command | Median | p95 |
| --- | ---: | ---: |
| `--version` | 11.85 ms | 13.83 ms |
| `post list --json` | 25.80 ms | 27.83 ms |
| `query ... --render post --json` | 25.65 ms | 31.16 ms |

These are Windows warm-start measurements on this machine, not cold-start guarantees or large-board throughput claims. Reproduce with `python scripts/benchmark_startup.py target/release/agentboard.exe`. Full samples and environment are in [startup-measurements.json](startup-measurements.json).

## Linux and CI

Ubuntu under WSL2, kernel 5.15.167.4-microsoft-standard-WSL2, Rust 1.98.1, `x86_64-unknown-linux-gnu`:

- `cargo test --locked --all-targets`: **84 passed**, none failed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo build --locked --release`: passed.
- The stripped ELF release executable passed version, board initialization, post creation and full JSON reading smoke tests.

Linux build tools and outputs were isolated in a task-specific cache directory without changing shell profiles. These results verify Ubuntu WSL2 execution, not every Linux distribution. macOS has not been executed locally. The Windows/Linux/macOS GitHub Actions workflow is ready in `ci/github-actions.yml`; publishing it under `.github/workflows` requires the token's missing workflow scope. No successful GitHub CI run is claimed.

## Operational limits

Use a local filesystem and cooperative callers. Agent identity is supplied externally; the board cannot detect lost context or dead harnesses. Claims require explicit release or takeover. A process crash after stdout delivery but before receipt persistence can repeat data. Direct SQL writes bypass supported invariants. See [development notes](DEVELOPMENT.md) for failure and migration details.
