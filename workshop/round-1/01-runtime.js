window.DECK = {
  id: '01',
  title: 'Language and packaging',
  subtitle: 'Proposed direction: Python, with an explicit installation tradeoff',
  slides: [
    {
      title: 'Choose for the work Agentboard actually does',
      html: `<p>An agent starts a command, opens a local board, asks SQLite for a precise answer, prints it, and exits. Several agents may do this simultaneously. The board remembers their progress between commands.</p>
      <p>The difficult work is defining reliable claims, histories, unread state, and compact output. A language does not settle those semantics.</p>
      <div class="callout">Provisional recommendation: Python for the first implementation. Prioritize inspectable SQL and clear command behavior while the product rules are still changing.</div>`
    },
    {
      title: 'Three credible choices, one practical tradeoff',
      html: `<table><thead><tr><th>Choice</th><th>Why it fits</th><th>What we accept</th></tr></thead><tbody>
      <tr><td>Python</td><td>Standard-library SQLite support. Direct mapping from commands to SQL.</td><td>A managed Python runtime on each machine.</td></tr>
      <tr><td>Go</td><td>Compiled executable releases. A strong option when installation must avoid Python.</td><td>A separate SQLite driver and its build requirements.</td></tr>
      <tr><td>Rust</td><td>Compiled releases and strict compile-time checks. SQLite can be bundled.</td><td>Explicit ownership rules and native SQLite build tooling.</td></tr>
      </tbody></table>
      <p>My judgment: Python keeps the first version easiest to revise. Go becomes my preference if a standalone download is required immediately.</p>
      <p class="source"><a href="https://docs.python.org/3.13/library/sqlite3.html">Python</a> · <a href="https://pkg.go.dev/database/sql">Go drivers</a> · <a href="https://github.com/rusqlite/rusqlite">Rust SQLite</a></p>`
    },
    {
      title: 'Concurrency belongs in the database contract',
      html: `<p>Alice and Bob claiming task 41 are separate processes, regardless of language. SQLite must decide the winner within a transaction: a group of changes that succeeds or fails together.</p>
      <p>SQLite's write-ahead log allows readers alongside a writer, but still permits only one writer at a time. We need short writes and a defined response when the database stays busy.</p>
      <div class="callout">No background service is required. Choosing Go or Rust would not remove this coordination problem.</div>
      <p class="source"><a href="https://sqlite.org/wal.html">SQLite concurrency rules</a></p>`
    },
    {
      title: 'What installation would look like',
      html: `<p>Proposed developer workflow, after packaging exists, from the repository folder:</p>
      <pre><code>uv tool install .
agentboard board.db alice task claim 41</code></pre>
      <p>uv installs the command into an isolated environment. Once its executable directory is on the shell's PATH, agents invoke Agentboard directly. Windows and Linux would both be tested.</p>
      <p>This accepts Python and installer setup. It does not assume you already use either. The command grammar remains illustrative, and today's repository is not yet installable this way.</p>
      <p class="source"><a href="https://docs.astral.sh/uv/guides/tools/">uv tool installation</a></p>`
    },
    {
      title: 'Keep future changes affordable',
      html: `<p>Separate command parsing, board operations, and SQL access. Keep stored content in ordinary SQLite tables, with documented schema changes. These boundaries also give parallel teams clear ownership.</p>
      <p>Packaging is relatively easy to change later. Rewriting the language is real work, even if the database survives. Stable command examples and JSON contracts would become compatibility checks for any replacement.</p>
      <div class="callout">Before release: test both systems, verify the linked SQLite includes the WAL-reset fix, and measure repeated commands. Python's version alone does not establish SQLite compatibility.</div>
      <p class="source"><a href="https://sqlite.org/wal.html#walreset">SQLite release requirement</a></p>`
    },
    {
      title: 'Decision: accept Python for the first version?',
      html: `<div class="callout">Recommended answer: yes, with an isolated CLI installation and direct SQLite access.</div>
      <p>This approves a language and an installation direction. The cost is requiring a Python runtime. If downloading a standalone executable is essential from day one, choose Go instead and validate its SQLite driver before feature work.</p>
      <p>Defer exact Python version, argument-parsing library, package publication, binary bundling, and HTTP framework. They do not need separate presentations now. The lead can bring back an exception if installation testing changes this recommendation.</p>`
    }
  ]
};
