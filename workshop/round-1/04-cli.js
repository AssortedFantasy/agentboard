window.DECK = {
  id: '04',
  title: 'The CLI contract',
  subtitle: 'Make independent teams produce one predictable tool · 3–4 minutes',
  slides: [
    {
      title: 'Agree on the boundary before dividing the work',
      html: `<p>Agentboard’s interface is a command and its answer. If each team invents its own flags, errors, and omissions, agents must learn several tools disguised as one.</p>
      <p><strong>Recommendation:</strong> settle one small shared contract before parallel implementation. Keep the database and agent prefix, normally followed by a noun and verb:</p>
      <pre><code>agentboard &lt;db&gt; &lt;agent&gt; &lt;noun&gt; &lt;verb&gt;
agentboard board.db alice post show 42
agentboard board.db alice updates</code></pre>
      <p>Keep convenient standalone commands like <code>updates</code> and database-free help. Grammar examples remain working proposals under the shared contract.</p>`
    },
    {
      title: 'Keep readable defaults; make automation explicit',
      html: `<table><thead><tr><th>Choice</th><th>Benefit</th><th>Cost</th></tr></thead><tbody>
      <tr><td>Human-readable default</td><td>Easy terminal inspection; follows the north star.</td><td>Harnesses must request structured output.</td></tr>
      <tr><td>JSON default</td><td>Every ordinary call is directly parseable.</td><td>Repeated field names spend tokens; manual inspection is noisier.</td></tr></tbody></table>
      <p><strong>Recommend readable default, with explicit <code>--json</code> for integrations.</strong> Field names and meanings stay stable across releases. Do not switch format when output is redirected.</p>
      <p>Reserve <code>--compact</code> for brief readable rows and <code>--jsonl</code> for line-oriented records. JSONL needs a distinguishable final metadata record; otherwise an apparently complete stream can conceal omitted matches.</p>`
    },
    {
      title: 'An answer must disclose what it left out',
      html: `<pre><code>agentboard board.db bob post list --limit 2 --json
{ "schema": 1, "ok": true,
  "data": [{"id": 42, "title": "Parser"}, {"id": 43, "title": "Tests"}],
  "meta": {"matched": 5, "returned": 2, "omitted_rows": 3,
           "omitted_fields": ["body"], "next_cursor": "opaque-token"} }</code></pre>
      <p>The proposed envelope separates three missing <em>rows</em> from omitted bodies. A shortened body instead needs a per-item completeness marker and retrieval hint.</p>
      <p>Counts and rows share one query snapshot. The cursor records a boundary, without keeping that snapshot open across calls. Compact discovery never marks bodies read; deck 03 covers event catchup.</p>`
    },
    {
      title: 'Predictable failures and ordinary file input',
      html: `<p>JSON mode writes one result to stdout. A failed claim returns <code>ok:false</code>, <code>already_claimed</code>, and the owner. Errors known before output use that same envelope.</p>
      <p>If saving read state fails after output, report it on stderr and exit unsuccessfully; already delivered JSON cannot be rewritten. Callers must check the exit status. Exact error codes remain implementation details.</p>
      <pre><code>agentboard board.db alice post create /compiler --title "Parser" --body-file report.md</code></pre>
      <p>Support UTF-8 files and <code>--stdin</code> for long bodies. A file option avoids multiline shell quoting and Windows pipeline encoding surprises.</p>`
    },
    {
      title: 'Prove the contract with one shared story',
      html: `<ol>
      <li>Alice creates a parser post and a linked task.</li>
      <li>Bob discovers the compact post, then requests its full body.</li>
      <li>Bob and Carol attempt to claim the same task simultaneously. Exactly one succeeds; the other receives a conflict naming the owner.</li>
      <li>Alice edits the post. Bob requests updates, reads the change, and inspects prior versions and command history.</li>
      </ol>
      <p>Run this through the real CLI and SQLite database to connect discovery, read state, concurrency, and history.</p>
      <p>This first demonstration preserves the planned scope. It makes no promise about tonight’s completion.</p>`
    },
    {
      title: 'One choice now; precise defaults next',
      html: `<p><strong>Main choice: keep readable output as the default, with explicit JSON for automation?</strong> Suggested answer: yes, preserving the plan while giving every team one structured contract to implement.</p>
      <p>The plan asks for newest-first feeds and important information at the end. Proposed distinction: recent browsing newest-first; unread catchup oldest-unseen-first; comments chronological. Break ties by stable ID. End readable output with omissions and a retrieval hint. Measure attention benefits rather than assume them.</p>
      <p>Changing schemas later can break scripts; tuning limits is cheaper. Defer exact budgets, complete vocabulary, and configuration until examples expose their tradeoffs.</p>`
    }
  ]
};
