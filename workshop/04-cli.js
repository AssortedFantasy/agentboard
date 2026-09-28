window.DECK = {
 id: '04', title: 'Posts as authored', label: 'Round 2 / accepted direction, illustrative output', duration: 'about 1 minute total',
 slides: [
  {title: 'The default response carries ordinary text', html: `<p>Readable output is the default. A model should not need to decode a JSON string to read this post:</p><pre><code>Post 42 · Parser investigation · revision 7

Use "strict" mode.
Keep the path C:\\work\\parser unchanged.

Next step:
Run the regression test.</code></pre><p>The body retains real line breaks, quotes, and backslashes. The heading identifies the object; its precise format can evolve.</p><p>Any truncation or summary substitution needs an explicit notice outside the body.</p>`},
  {title: 'Structured output stays optional', html: `<p>Harnesses can explicitly request JSON when they need fields. That format necessarily escapes string content. It should serialize a post body once, without embedding another JSON document in the body field.</p><p>For ordinary reading, preserve body text. For exact extraction, a proposed body-only mode can omit headings and footers. Those are separate uses, not a reason to force a wrapper on every call.</p><p class="callout">No new approval request here. Next, iterate on actual create, search, read, and update output examples. The CLI will need more than one broad presentation.</p>`}
 ]
};
