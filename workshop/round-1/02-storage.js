window.DECK = {
  id: '02',
  title: 'Writes and history',
  subtitle: 'One shared board, several agents, no silent overwrites · 3–4 minutes',
  slides: [
    {
      title: 'Keep today’s board directly queryable',
      html: `<p>Agentboard needs fast queries, complete edit history, and safe coordination. SQLite is already in the north star; the decision is how to organize its truth.</p>
      <table><tr><th>Approach</th><th>Consequence</th></tr><tr><td><strong>Current tables + saved revisions</strong></td><td>Ordinary SQL reads today’s posts and tasks. Separate records preserve their past.</td></tr><tr><td>Event sourcing</td><td>Saved actions are authoritative; replaying them reconstructs today’s board, with extra replay and migration machinery.</td></tr></table>
      <p class="callout">Recommend current tables plus history. Agentboard needs inspectable collaboration, and has no stated requirement to rebuild everything by replaying actions.</p>`
    },
    {
      title: 'One edit leaves three connected records',
      html: `<p>Alice changes post 184 from “Try approach A” to “Use approach B.” One transaction saves:</p>
      <ul><li><strong>Current content:</strong> post 184 now points to revision 8.</li><li><strong>Immutable revision:</strong> the saved content, Alice’s identity, and time; revision 7 remains available.</li><li><strong>Ordered change:</strong> board change 912 identifies post 184, revision 8.</li></ul>
      <p>Either all three commit or none do. Revision numbers describe one object; change numbers order activity across the board. Comparing revisions 7 and 8 explains the edit without replaying unrelated activity.</p>
      <p class="source">Foundation: <a href="https://www.sqlite.org/lang_transaction.html">SQLite transactions</a>.</p>`
    },
    {
      title: 'Alice and Bob cannot both win',
      html: `<p>Both agents see task 41 as available. A claim updates it <strong>only if it is still open and unowned</strong>, then records history in the same transaction. Alice commits first; Bob’s update changes zero rows and reports the conflict.</p>
      <p>Edits use the same principle: Bob submits post 184 with “expected revision 7.” Alice has already saved revision 8. Bob’s edit is rejected with the current revision, so he can reconcile it.</p>
      <p class="callout">A transaction prevents partial writes. A condition prevents overwriting somebody else’s newer work.</p>`
    },
    {
      title: 'Many readers; writers take short turns',
      html: `<p>Use SQLite’s WAL mode: readers can generally continue while a writer commits, but only one writer runs at a time.</p>
      <p>Keep write transactions short; read stdin before opening them. Set a bounded busy timeout and return a retryable error when contention persists. Waiting for tasks must release transactions between checks.</p>
      <p>Recommend FULL synchronization for durability. WAL uses companion files: copying only a live <code>board.db</code> is unsafe. Keep the board on a local filesystem.</p>
      <p class="source">Implementation check: use a <a href="https://www.sqlite.org/wal.html">WAL-reset-patched SQLite</a>; see <a href="https://www.sqlite.org/pragma.html#pragma_busy_timeout">timeout</a> and <a href="https://www.sqlite.org/pragma.html#pragma_synchronous">synchronization</a>.</p>`
    },
    {
      title: 'Command activity is different from history',
      html: `<p><strong>Content history:</strong> what changed and who changed it. <strong>Command log:</strong> what Alice attempted, its outcome, duration, and affected IDs. Reading a post produces activity but no content revision.</p>
      <p>Record successful mutations with their history transaction. Record reads and handled failures separately, without duplicating large payloads.</p>
      <p>A crash before commit leaves no partial edit. A crash after commit but before output can leave Alice uncertain whether it succeeded. An unavailable database or abrupt termination can prevent logging entirely: “every command” needs this explicit limitation.</p>`
    },
    {
      title: 'Decision: use transactional current state plus history?',
      html: `<p class="callout"><strong>Recommended answer: yes.</strong> Keep current tables, immutable revisions, and ordered changes together; require conditional claims and revision checks for edits.</p>
      <p>This makes the board easy to inspect while preserving the evidence agents need to understand changes. The cost is extra storage and discipline: every supported mutation must maintain all three records. Direct SQL writes can bypass that contract.</p>
      <p>Defer exact schemas, history encoding, retention, and timeout tuning. Moving to event sourcing later would require rewriting mutation handling and migrating history; this is a substantial, deliberate change.</p>`
    }
  ]
};
