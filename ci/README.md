# Activate GitHub Actions

`github-actions.yml` is the complete Windows/Linux/macOS test and release-build workflow. It is stored here because the GitHub OAuth credential used to publish v1 lacks the `workflow` scope. GitHub rejected a push containing `.github/workflows/ci.yml`. The workflow has not executed on GitHub.

After granting that permission with `gh auth refresh -h github.com -s workflow`, move this file to `.github/workflows/ci.yml`, commit, and push. It runs formatting, Clippy, the full test suite and release builds, then uploads each platform's executable. No secrets are needed by the workflow.

Equivalent local gates:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
```
