# Agentboard workshop decisions

Updated after user feedback, 27 September 2026. These notes distinguish user direction from proposals. The north star remains `../plan.txt`.

## 01 — Language: Rust accepted

The user approved Rust from the start. Build the implementation in Rust rather than first building a Python prototype and scheduling a port.

The user's priorities are a fast CLI, low process startup cost, portability, and fewer runtime failures. These informed the Rust choice. The earlier Python recommendation is superseded.

Implementation direction: use Cargo's test runner with focused behavior tests, real SQLite integration tests, and tests that launch the CLI. Dependencies and release targets remain implementation details to resolve.

## 02 — Update semantics: direction accepted

Keep current revision information and preserve earlier revisions as history. Atomic conditional updates are expected behavior. The user accepts this general direction; another presentation explaining basic transactions is unnecessary.

Exact physical schema, migrations, logging policy, and retry details remain implementation work. Acceptance of the direction does not approve every detail in the first deck.

## 03 — Read tracking: direction recorded, exact semantics deferred

- Net diffs are the default. Intermediate edits remain available in history.
- Automatic tracking is mandatory. Do not require acknowledgement calls.
- Prefer a distinction between an object being seen/discovered and its content being read. This updates the earlier discussion against fragmented field-level tracking; it does not prescribe separate receipts for every field.
- The timestamp suggestion concerned richer state and observability compared with a boolean, not a request to use wall-clock time as the diff boundary. The user noted command-log timestamps may make a separate read timestamp unnecessary.
- Search results should not be categorically excluded from automatic tracking. Exactly how partial results affect a future diff is unresolved.
- An optional peek mode can wait.

The user explicitly deferred exact semantics, leaning toward tracking seen versus read. Scanning titles should not cause unchanged items to repeatedly reappear in updates solely because their bodies remain unread. Seeing a title must not imply knowledge of the body. Revisit concrete behavior with CLI examples; neither a per-forum discovery cursor nor a particular per-object schema is approved yet. Exact handling of partial results, timestamps, and diff baselines remains deferred.

## 04 — CLI: direction accepted, examples still need iteration

Human-readable default output is favored. Posts should appear as authored, without requiring the model to unpack escaped JSON strings or multiple layers of serialization. JSON remains an explicit integration format; it is not the default model-facing reading experience.

Treat the CLI as a first-class surface to iterate with concrete examples. Grammar, exact envelope fields, numeric exit codes, truncation budgets, and configuration were not approved as a package. Do not present them as settled.

## 05 — Tasks as posts: provisional direction

Work items are posts with optional structured task state, such as status, owner, and dependencies. Reuse the post body, comments, tags, links, and history. The user found this reasonable and related it to familiar issue assignment and blocking behavior. Validate the abstraction with a small implementation example before treating its exact design as settled.

Task operations can produce automatically generated discussion entries and task information in the post display. Lead implementation proposal: update structured task state and record its event atomically; render the event as a system entry, without parsing comment prose to infer ownership or rewriting the author's body.

Agents are ephemeral. Exact abandonment and takeover semantics remain open. An agent may use comments and recent activity to judge whether work is abandoned, but silence cannot guarantee that another agent has stopped working. Persistent claims, expiry, and explicit takeover have not been approved as a policy.

## 06 — References and dependencies: direction accepted

Use `#id` to reference posts, including posts with task state. Automatically resolve references and expose backlinks, with generated activity entries where appropriate.

The user suggested a post backlink entry such as "mentioned in <title>". For comments, prefer compact generated information such as "mentions: #12, #38" rather than polluting the comment body. Exact comment presentation, placement of incoming backlink notices, and reference behavior after edits remain to be worked out. Generated information should remain separate from authored text.

Task dependencies form a separate, explicitly managed directed acyclic graph. A textual reference does not create a dependency. Provide a dependency command accepting multiple IDs, and consider allowing dependencies during task creation. The user clarified that "dependents" was a typo: `task depend 41 38 39` means task 41 depends on prerequisites 38 and 39. Exact command spelling remains subject to CLI iteration.

The intended coordination use includes letting an agent block in a simple, slowly polling wait command. Exact wait conditions and behavior remain to be specified.

The user explicitly approved both `depends` and `blocks` directions at creation time and in commands modifying existing tasks, to avoid extra invocations. Both express the same graph edges: task 41 depending on task 52 is equivalent to task 52 blocking task 41. Accept multiple referenced IDs. Support combining both directions in a creation request when needed. Exact spelling remains subject to CLI iteration; illustrative forms are `task depend 41 52`, `task block 52 41`, and creation flags `--depends-on 38 --blocks 41`. Apply a requested batch of dependency changes atomically and reject cycles rather than leaving a partially applied graph (lead implementation rule).

## 07 — Waiting on prerequisites: working direction

For an agent waiting until a task's prerequisites are satisfied, the user supports cancellation waking the waiter immediately and suggests that ordinary completion should wake it only once all prerequisites are complete. The lead recommends this as the default: intermediate completions remain recorded but do not end the wait; any cancelled prerequisite ends the wait with a cancellation reason; all completed prerequisites end it with a ready reason. A timeout ends the wait with a timeout reason. Waking on cancellation does not satisfy or remove the dependency.

"Immediately" means on the next polling check within the simple polling implementation, not push delivery. Conditions already true when the wait starts should return without sleeping. Exact polling interval, output format, and handling of changes to dependencies during a wait remain implementation details to resolve.

The user also envisions waiting across subscribed activity sources, analogous to waiting for any of several events. Treat task readiness and subscription activity as potential inputs to the broader wait facility. This is a behavioral direction, not a requirement to use Linux epoll. Exact subscription/wait mechanics are deferred; the user requested a return to more significant architectural and product choices.

## 08 — Query interface: full SQL direction accepted

The user likes full SQLite SQL queries, with results rendered as Agentboard content rather than merely dumped as tuples. Documented SQL views sound reasonable to the user, although the exact exposed schema remains to be designed. This supersedes the previous recommendation to use only composable CLI filters initially.

The user accepted moving forward after the recommendation for full SQL expressiveness with an explicit rendering contract and independent configurable limits. Object-rendering queries select IDs for a declared target; validate their result shape and give an actionable error for incompatible projections such as `SELECT *`. Tabular query mode can accept arbitrary result columns. Keep ordinary rendering, omission notices, and automatic tracking in Agentboard rather than returning raw object tuples.

Enforce object/row counts and total output size independently of SQL `LIMIT`, plus a configurable query execution budget. Missing `LIMIT` must not dump the board into context. Allow deliberate larger requests, disclose omissions, and avoid requiring an expensive exact result count. Exact defaults, syntax, and implementation details remain to be chosen.

## 09 — Agent identity: external, ephemeral agents; direction accepted

The user envisions names supplied out of band by a prompter or parent agent, such as `codex-lead-1`, `codex-sub-1`, and `claude-lead-1`. Agentboard must remain independent of the harness. Identities primarily represent individual agents that come and go, rather than persistent roles automatically inherited by replacements.

The user agreed to move forward with externally supplied identities and an optional name-allocation command, with a caller-provided prefix and generated suffix checked for uniqueness within the board. One identity is reused across a continuing agent's commands; a fresh replacement should allocate a new identity. Retain departed agents' authorship and history. Do not add a separate session system by default.

Agentboard cannot infer whether two invocations with the same supplied identity share a context window or retained knowledge. Context-loss recovery remains explicit; full reads or a future reading-state reset can help, but the harness/caller must identify the need. Do not infer agent death or context continuity from inactivity alone. Exact registration syntax and reset behavior remain implementation details.

## 10 — Discussion layout: chronological default accepted

The user chose a chronological timeline as the normal discussion view. Preserve comment IDs and explicit reply-to relationships so alternative commands can provide thread-focused or tree-like views. Threaded reading is an alternative view of the same discussion, not a separate content model. Exact command syntax remains to be designed.

## 11 — Web inspection: included in v1

The user explicitly included the read-only web interface in v1. Keep it extremely barebones and focused on presenting content: ordinary links, simple borders or square boxes for demarcation, and minimal stylistic flair. Avoid decorative dashboards or elaborate frontend styling.

Use the existing content/query model to present posts, chronological discussions, task state, and activity. Keep writes in the CLI and keep the server optional for ordinary CLI operation, consistent with the north star. Exact page structure and frontend implementation are technical-lead details rather than additional style decisions for the user.

## 12 — Notifications: full flow included in v1

The user explicitly requires the whole notification flow in v1, including subscriptions, feeds, inboxes, durable notifications, and integration with waiting. Agentboard should be capable of replacing the message-post coordination agents otherwise perform through harness-specific facilities. Preserve harness independence while making attention and delivery behavior a core product concern.

Automatic subscriptions are part of the intended direction. The exact triggers and rules remain to be designed; the user has not approved a particular list. Account for replies, mentions, assignments, changes to followed work, and dependency outcomes in end-to-end validation. The earlier lead suggestion to defer richer notification or separate inbox behavior is superseded.

## Delivery principle — validate breadth rather than routinely cut scope

The user identifies validation and good decisions, rather than feature implementation breadth, as the primary bottlenecks for agent-driven development. Do not use "not in v1" as a default reason to omit planned capabilities. Implement complete workflows so they can produce validation feedback. Sequence implementation for integration and testing, without silently converting that sequencing into feature exclusions. Any proposed omission needs a concrete product or technical reason and explicit discussion.

Existing explicit deferrals of exact semantics still stand. This direction does not silently approve every speculative feature or previously proposed implementation detail.

## Review process

The user has ended the presentation workflow. Surface one substantive decision at a time in chat, explain the practical tradeoff, and recommend an answer. Discuss it and record the outcome before moving to the next item. Do not create further presentations unless requested.

Keep routine engineering choices with the technical lead. Distinguish accepted choices from suggestions; do not infer approval from a question or silence. Once the consequential decisions are sufficiently clear, move into implementation with the user. Rust is approved; exact seen/read semantics are deliberately deferred.
