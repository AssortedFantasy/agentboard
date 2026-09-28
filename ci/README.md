# CI cost and execution controls

The workflow is maintained in [`.github/workflows/ci.yml`](../.github/workflows/ci.yml).

It uses only standard GitHub-hosted Linux, Windows and macOS runners, which are free for public repositories. A job-level visibility guard skips compute if the repository becomes private. Each job has a 20-minute timeout. The workflow uses read-only permissions, pinned action revisions, and no secrets.

Pull requests to `main` run once per relevant update; push builds run only on `main`, avoiding duplicate branch-push and PR runs. Documentation-only pull requests and main pushes do not trigger CI. GitHub evaluates the cumulative PR diff, so a documentation-only follow-up on a PR that changes source can still trigger a run. New runs cancel superseded runs for the same PR/branch. A failing platform cancels unfinished siblings. Manual runs are also available.

Formatting and Clippy run once on Linux. All three platforms run the tests in release mode, including CLI and HTTP subprocess tests, so a second debug build is unnecessary. No artifacts or caches are uploaded, preventing persistent storage accumulation. The small dependency graph builds quickly without a remote cache.

Keep the account's Actions product budget at $0 with **Stop usage** enabled. This account setting is outside Git and should be rechecked if ownership or billing configuration changes. Free compute does not extend to paid larger runners, private-repository overages or chargeable storage; do not introduce those without reviewing the spending controls.

References: [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions), [budget controls](https://docs.github.com/en/billing/how-tos/set-up-budgets).

Equivalent local gates:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --release --all-targets
```
