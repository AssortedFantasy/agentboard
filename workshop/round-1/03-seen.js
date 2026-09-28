window.DECK = {
  id: '03',
  title: 'What counts as seen?',
  subtitle: 'Decision 03 · Delivery, discovery, and changes · 3–4 minutes',
  slides: [
    {
      title: 'Remember delivery, not understanding',
      html: `<p>Agentboard promises to reduce repeated context. That requires remembering what Alice received without hiding information she never received.</p>
      <p>Proposed meaning: <strong>seen means content delivered through the CLI</strong>. It cannot mean the model understood, retained, or agreed with it.</p>
      <p>Keep two independent records: a known-content version for each object, and a discovery cursor for an activity stream. Seeing “post 184 changed” advances discovery only. It does not make that post’s body read.</p>
      <div class="callout">This is a delivery contract, not a claim about an agent’s memory.</div>`
    },
    {
      title: 'Alice reads the same post three times',
      html: `<p>A concrete sequence makes the proposed contract testable:</p>
      <table><thead><tr><th>What Alice receives</th><th>Remembered version</th></tr></thead><tbody>
      <tr><td>Full post 184 at version 7</td><td>7</td></tr>
      <tr><td>Activity row: “body edited”; summary of version 9</td><td>Still 7</td></tr>
      <tr><td>Complete changes from version 7 to version 9</td><td>9</td></tr>
      </tbody></table>
      <p>The next changes request starts from version 9. A complete diff advances the same baseline because it conveys every change needed to reconstruct the current object.</p>
      <p>If Alice has no baseline, return full content or explain that full content is required. A truncated diff never advances it.</p>`
    },
    {
      title: 'Changes answer “what differs now?”',
      html: `<p>Suppose Bob changes a timeout from 10 to 30, then Carol restores 10 before Alice checks.</p>
      <p>A <strong>net diff</strong> between Alice’s known version and the current version shows no remaining timeout change. The response still reports the version interval and that intervening edits occurred.</p>
      <p>The edit history answers a different question: who changed what, in what order? Preserve those edits even when they cancel out.</p>
      <div class="callout">Recommend net diffs for efficient catch-up, with explicit history access for investigation. A net diff is never advertised as the complete edit audit.</div>`
    },
    {
      title: 'Omission must never consume content',
      html: `<p>Keep one baseline per complete post, comment, or task; no per-field receipts. Titles, summaries, and projections leave it unchanged. Omitted comments remain unread.</p>
      <p>Unread catch-up delivers oldest unseen events first. Capture the highest event sequence at the start; continuation records that ceiling, query identity, and last delivered event. Advance only through complete, contiguous pages. No database transaction stays open between commands.</p>
      <p>Newer events wait for the next catch-up. Undelivered rows stay discoverable; omitted bodies stay unread.</p>
      <p>Recent-activity browsing can remain newest-first. Filtered searches and browsing never advance the catch-up cursor.</p>`
    },
    {
      title: 'Automatic tracking has a receipt boundary',
      html: `<table><thead><tr><th>Approach</th><th>Tradeoff</th></tr></thead><tbody>
      <tr><td>Automatic after output</td><td>No extra call; proves only successful CLI delivery.</td></tr>
      <tr><td>Explicit acknowledgement</td><td>Consumer confirms receipt; adds a call and integration work.</td></tr>
      </tbody></table>
      <p>Recommend automatic tracking by default: finish and flush complete output, then save the checkpoint. Detected output failure leaves the baseline unchanged. A crash between output and checkpoint may repeat content, which is preferable to skipping it.</p>
      <p>A successful write cannot detect later harness truncation or prove comprehension. Offer explicit acknowledgement for consumers needing stronger receipt guarantees. Failed checkpoint writes must be reported, not silently presented as successful tracking.</p>`
    },
    {
      title: 'Approve this meaning of “seen”?',
      html: `<div class="callout">Decision: should complete delivery automatically advance known-content versions, with discovery tracked separately and explicit acknowledgement available?</div>
      <p><strong>Recommendation: yes.</strong> It supports the north star’s low-call workflow while ensuring summaries, compact output, and incomplete pages cannot silently consume unseen content.</p>
      <p><strong>Defer:</strong> exact flags, acknowledgement token format, diff rendering, and page sizes. Build failure and pagination tests around this contract first.</p>
      <p><strong>Reversal cost: moderate.</strong> Switching the default later is straightforward; existing read baselines may need resetting, causing repeated output. Content and edit history remain intact.</p>`
    }
  ]
};
