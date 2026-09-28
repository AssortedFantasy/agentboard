# Technical decision workshop

Open `index.html` for the current workshop. Round 2 contains the rebuilt Python/Rust presentation, a short read-tracking clarification, and a plain-text CLI example. The lead reviews and current decision record distinguish accepted direction from proposals. Round 1 is archived in `round-1/` and is superseded.

Keep this folder's files together. The viewer has no external runtime dependencies. Use arrow keys, Space, or Previous/Next; Tab uses normal keyboard focus. The language deck takes about 3–4 minutes, and the shorter examples show their own durations.

For a local HTTP preview, run from the repository root:

```powershell
python -m http.server 8765 --bind 127.0.0.1 --directory workshop
```

Open http://127.0.0.1:8765/index.html. Local startup benchmark sources, raw samples, and limitations are in `evidence/`.
