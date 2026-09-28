window.DECK = {
  id: '01',
  title: 'Rust now, or Python then Rust?',
  label: 'Round 2 / language decision / revised proposal',
  subtitle: 'Revised recommendation: build the first usable slice in Rust',
  slides: [
    {
      title: 'The choice is when to pay for Rust',
      html: `<p>You accept Python as an experiment, not the intended destination. That changes the decision: compare a disposable Python prototype followed by an agent-led port with building the first usable slice directly in Rust.</p>
      <p>The first deck overvalued easy revision and missed repeated startup, runtime failures, and distribution. With those criteria explicit, my recommendation changes to Rust first.</p>
      <div class="callout">Start with a narrow working CLI. Rust first does not require committing to every feature or database rule before coding.</div>`
    },
    {
      title: 'Startup is paid on every invocation',
      html: `<p>Measured here on Windows: 50 warm launches per variant, interleaved. Timing includes process creation through exit. Rust and Python perform the same tiny argument/output task.</p>
      <table><tr><th>Probe</th><th>Median</th><th>95th percentile</th></tr><tr><td>Optimized Rust</td><td>12.1 ms</td><td>16.8 ms</td></tr><tr><td>Python</td><td>39.0 ms</td><td>55.6 ms</td></tr><tr><td>Python + SQLite import*</td><td>54.3 ms</td><td>76.6 ms</td></tr></table>
      <p>Rust has lower launch overhead here. This is not an Agentboard benchmark: no queries, diff generation, installed launcher, or cold starts. Large queries and output can change the balance.</p>
      <p class="source">*Additional import only; no equivalent Rust SQLite dependency. Python 3.12.1, Rust 1.98.1. <a href="evidence/README.md">Method and limitations</a> <a href="evidence/startup-results.json">Raw measurements</a></p>`
    },
    {
      title: 'Rust catches more before agents run it',
      html: `<p>Rust rejects many type and ownership mistakes at compilation. Recoverable failures use <code>Result</code>, an explicit success-or-error value. But <code>unwrap()</code> on an error can panic and stop the command. Compiling successfully does not prove correct behavior.</p>
      <p>Python can handle exceptions cleanly. Type annotations require a separate checker; Python does not enforce them. Rust makes type checking mandatory, but neither language proves SQL correctness, valid user input, or complete error handling.</p>
      <p>Both paths call SQLite's native C code. Rust's guarantees do not eliminate bugs across that boundary. Either implementation needs deliberate error messages and failure tests.</p>
      <p class="source"><a href="https://doc.rust-lang.org/book/ch09-02-recoverable-errors-with-result.html">Rust errors</a> · <a href="https://docs.python.org/3/library/typing.html">Python typing</a> · <a href="https://docs.python.org/3/tutorial/errors.html">Exceptions</a></p>`
    },
    {
      title: 'Portable source is different from portable installation',
      html: `<p>Python source can run across systems with a suitable interpreter and dependencies. That shifts compatibility work onto runtime setup. Rust releases need a build for each supported operating system and processor architecture. One Windows executable is not a Linux executable.</p>
      <p>With Rust's <code>rusqlite</code> bundled option, releases can include a controlled SQLite version instead of relying on the user's SQLite installation. Building that dependency requires C tooling.</p>
      <div class="callout">Recommendation: build and test Windows and Linux releases. Verify their dependencies before promising a self-contained download.</div>
      <p class="source"><a href="https://doc.rust-lang.org/rustc/platform-support.html">Rust targets</a> · <a href="https://github.com/rusqlite/rusqlite">SQLite bundling</a></p>`
    },
    {
      title: 'An agent-led port still has a compatibility bill',
      html: `<p>A Python prototype earns its cost when it answers a specific unsettled product question. Agents can translate implementation, but someone must verify that the replacement preserves observable behavior.</p>
      <p>That includes old database migrations, identifier handling, shell escaping, Unicode, JSON shapes, output ordering, and exit codes. A plausible-looking Rust rewrite can disagree quietly on any of these.</p>
      <div class="callout">If choosing Python first, freeze a small set of command examples and expected results before porting. Keep prototype databases disposable unless migration compatibility is explicitly included.</div>`
    },
    {
      title: 'Decision: build the first usable slice in Rust?',
      html: `<div class="callout">Recommended answer: yes. Prioritize repeated invocation, compile-time checks, and controlled releases without scheduling a second implementation.</div>
      <p>The cost is native build setup and compile cycles during iteration. The benefit is testing the intended deployment from the beginning. Treat unchecked panics in normal input paths as review failures.</p>
      <p>Python remains reasonable for a bounded experiment, such as comparing diff output on representative threads. Stop when that question is answered. A first Rust slice should then measure real reads and writes, validate ordinary failures, and run on the proposed Windows/Linux targets. Whole-application speed remains unmeasured.</p>`
    }
  ]
};
