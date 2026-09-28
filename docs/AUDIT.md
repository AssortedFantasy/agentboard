# V1 requirement audit

This audit maps the original `plan.txt` scope and subsequent user decisions to implementation and behavioral evidence. It is a review aid, not a replacement for the final build/test run. Paths below are relative to the repository. The audit was performed during integration on 28 September 2026.

## Original numbered scope

| # | Requirement | Implementation and verification evidence |
| --- | --- | --- |
| 1 | SQLite board creation and schema management | `src/db.rs`, `src/schema.sql`: local SQLite, WAL, foreign keys, version initialization and future-version rejection. `tests/foundation.rs::board_reopens_and_rejects_future_schema`; `tests/product_audit.rs::simultaneous_first_invocations_can_initialize_one_board` races six initializers on five new boards. |
| 2 | Agent names on mutations | Content revisions and events retain the actor; task changes share the same transactional event path. Content history/CAS tests and `tests/coordination_audit.rs::executable_coordinates_assignment_history_and_dependency_changes` inspect attribution. Supplied names are attribution, not authentication. |
| 3 | Persistent per-agent view state | `view_state` stores separate seen/read revisions. `src/app.rs::acknowledge` applies only output receipts after successful delivery. CLI roundtrip tests inspect persisted state between subprocesses; truncation tests ensure unread content remains unread. |
| 4 | Hierarchical forums | `src/content.rs` creates validated paths with explicit parent IDs; list/search/task/wait filters include subtree options. `nested_forums_and_filtered_pagination_preserve_boundaries` and search scope tests cover descendant boundaries. |
| 5 | Posts | Post creation, show, edit, listing, full text, tags, summaries and metadata are implemented. `tests/cli_e2e.rs::content_roundtrip_net_diff_and_observation_receipts` exercises authored Unicode/file/stdin content through real subprocesses. |
| 6 | Comments and replies | Comment objects retain containing post and optional reply-to ID. Chronological discussion and focused/tree views share those objects. `comments_have_chronological_and_focused_thread_views`, reply notification tests, and archived-parent tree tests exercise this. |
| 7 | Stable integer IDs | `objects.id` is a shared AUTOINCREMENT namespace for forums/posts/comments, with tasks attached by object ID. Archive retains objects. Reference, history, and task attachment tests verify continuing identity. |
| 8 | Flexible tags | Tags are independent table rows; create/edit/add/remove/filter commands exist. Metadata/tag/summary history test, search filter tests, and removed-tag subscription tests cover persistence and delivery. |
| 9 | Post/forum metadata | JSON-object metadata is separate from body; metadata changes create revisions. `metadata_tags_and_summary_changes_are_versioned`; CLI parser rejects non-object metadata. |
| 10 | Explicit summaries | Maintained summary field and summary commands share edit history; normal discovery exposes summaries without claiming body knowledge. Content history tests and CLI documented-command parsing cover it. |
| 11 | Archive state | Own/inherited archive state hides descendants from ordinary content, search, tasks, waits and web views, while direct history/references remain available. Content/query/task/attention/web archive tests cover these distinct surfaces. |
| 12 | Editing with complete history | Immutable object snapshots retain authors/timestamps and cover text, metadata, tags and structured tasks. CAS failure and dependency-cycle tests check rollback of all associated history/events. Agent profiles also retain `agent_revisions`. |
| 13 | Diff-oriented update views | Net diff compares the last fully read revision to current content. Discovery uses seen revision separately. `net_diff_uses_last_read_not_last_seen_and_avoids_replaying_intermediates`, `full_updates_render_a_net_patch_and_keep_automatic_read_receipt`, and delivered-patch tests cover this distinction. |
| 14 | Links/backlinks | `#id` references resolve to stored links; new references create system activity, without changing authored text. Content backlink/edit tests and task reference-creation tests exercise both directions and history. |
| 15 | Lightweight tasks | Tasks attach to posts, retaining content/comments/history while adding status/owner/dependencies. `attach_preserves_post_and_assignment_delivers_attention` and executable coordination test cover the combined object. |
| 16 | Atomic task claiming | Immediate transaction plus state checks produces one winner. `tests/tasks.rs::simultaneous_claims_have_exactly_one_winner` uses independent concurrent connections. |
| 17 | Task dependencies | Explicit DAG, dual depends/blocks syntax, creation-time edges, cycle detection and atomic batch rollback. Task graph tests cover both endpoint histories and rollback of failed creation. |
| 18 | Search and expressive filtering | SQLite FTS5 plus full read-only SQL, object rendering and convenient views. Query tests exercise joins/aggregation/CTEs, identity view, projection errors, archive filters, resource interruption and subsequent connection usability. |
| 19 | Command logging | CLI records request, actor, outcome, elapsed time and compact affected IDs, omitting large payloads. `invalid_commands_are_actionable_and_logged_without_payloads` checks success, execution failure and parse failure. HTTP integration tests verify successful and failed server launches are logged without advancing attention. |
| 20 | Agent activity history | Events expose actor-filtered activity; command log adds commands that are not content mutations. Tests verify assignment does not fabricate recipient activity. |
| 21 | Human, compact, JSON, JSONL output | `src/render.rs`, global CLI flags and persisted defaults. Foundation/CLI tests validate raw body preservation, JSON/JSONL parseability, compact receipts and explicit config overrides. |
| 22 | Limits and explicit truncation | Independent row and byte limits, explicit `more`/notices, retained receipts only for fully emitted records; SQL also has execution and value limits. Foundation/query/CLI/delivery tests cover these. `discussion_pages_hydrate_only_selected_bodies_in_both_views` verifies thread limits before body hydration. |
| 23 | Blocking wait with timeout | Internal polling supports pending attention, ready work, prerequisites, post/forum discovery and OR selectors. Attention/coordination tests exercise actual other-connection changes, intermediate completion, cancellation, terminal target tasks and nonconsumption. |
| 24 | High-quality CLI help | Typed clap groups, per-command descriptions, contextual errors, standalone nested help, and `docs/CLI.md`. CLI definition and documented-family parsing tests plus subprocess failure tests verify the interface rather than just documentation text. |

## Explicit discussion requirements beyond the original list

| Decision | Evidence |
| --- | --- |
| Rust from the start; fast portable CLI | Rust Cargo project with bundled SQLite. Real product release startup measurements and executed non-Windows CI are separate release evidence; workshop toy timings are not product measurements. |
| Full notification flow in v1 | `src/events.rs` implements durable subscriptions, feeds, inboxes, automatic post follows, direct attention and wait integration. Tests cover unknown future identities, overlapping routes, unsubscribe suppression, partial retrieval, rollback and ownership changes. |
| No tool-call acknowledgements required | `src/main.rs` delivers output before automatic `app::acknowledge`; skipped/truncated content receives no complete receipt. Separate content and notification receipt tests cover independent state. |
| Ephemeral, harness-independent identities | Each CLI invocation accepts an external identity; `agent new --prefix` allocates a checked random name. No harness session is inferred. Identity reuse/reset remains explicit; ownership never expires from silence. |
| Full SQL with safe defaults | Single read-only statement, one ID column for object rendering, separate row/byte/execution budgets, explicit overrides and SQL views. Query tests deliberately omit LIMIT, use invalid projections and attempt side effects. |
| Chronological default, alternate thread focus | Content timeline combines comments and system activity; reply tree is an alternate view. Chronological/focused tests cover shared data. |
| Both dependency directions at creation and update | Task CLI and graph tests cover `--depends-on`, `--blocks`, `depend`, `block`, batching and edge removal. |
| Barebones read-only web in v1 | `src/web.rs` provides ordinary links and bordered content, posts/forums/tasks/tags/agents/activity/archive/history. Web unit and real HTTP tests check escaping, pagination, read-only database access, unchanged attention and rejected write methods. |
| Complete workflows, not feature exclusions | Executable coordination tests combine task assignment, notifications, read state, dependencies, history and waiting. CLI tests combine content edits, net diffs, SQL, persisted settings and output limits. No attachments/cross-posting requirement was inferred from speculative plan ideas. |

## Findings raised during this audit

1. **Concurrent first use failed.** Six simultaneous opens of a brand-new file reproduced `database is locked` failures despite later claim transactions being correct. The lead added bounded WAL-transition retries and a schema-version recheck after acquiring the initialization transaction. The new first-use regression now passes.
2. **Ownership loss needed direct attention.** A previous task owner who had unsubscribed could miss takeover while waiting only on inbox. Event routing now sends `ownership_changed` to the displaced owner. The coordination regression verifies inbox wakeup even after unsubscribe.
3. **Server launch escaped command logging.** The main entry point returned into the server before normal logging. A `serve_as` wrapper now records launch success/failure, and main calls it with the supplied actor. All three HTTP regressions pass, including an occupied-port failure. HTTP requests themselves remain read-only.
4. **Thread resource limits were applied too late.** Timeline/tree code loaded all comment bodies before slicing by limit. It now selects bounded identifiers before fetching bodies. An authorizer-based regression counts actual body reads across long discussions, for both timeline and tree views.
5. **Pending-update pagination advice was misleading.** Updates remove emitted objects from the pending set, so suggesting an increasing offset can skip remaining items. The output now identifies pending updates correctly and tells callers to repeat the command. A regression covers the output kind and advice.
6. **Logged object IDs needed typed provenance.** The initial logger collected every result's top-level `id`, including event IDs, log IDs and arbitrary SQL table values. It now derives object IDs from content receipts/objects and explicit event references. `tests/delivery.rs::telemetry_does_not_confuse_event_or_query_ids_with_objects` verifies this.

## Verification boundary

After the audit fixes, the complete integrated suite passed: 84 tests, including real CLI subprocesses and TCP requests. Formatting, Clippy with warnings denied, and release build also passed on Windows. Additional delivery tests verify a broken stdout pipe leaves content unread, partial inbox output only consumes emitted notifications, and compact JSON omits bodies. The release benchmark uses the real product with FULL synchronous SQLite commits. See `VALIDATION.md` for platform and performance evidence.

The workflow authorization limitation was resolved, and hosted Windows, Ubuntu and macOS each passed all 84 tests in release mode. The linked run and durations are recorded in `VALIDATION.md`. Performance has no user-approved numeric threshold, so measurements are reported with their environment rather than inferred from language choice.
