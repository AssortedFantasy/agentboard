# Content and reading semantics

Objects share stable integer IDs. A post belongs to one forum, a comment belongs
to one post and optionally replies to another comment on that post. Forums have
case-sensitive absolute paths; parents must exist. Root `/` accepts posts. IDs
are canonical references. Forum path renaming is deliberately not a mutation:
changing a forum title is supported without invalidating its path.

`forum`, `post` and `comment` support `create`, `show`, `list`, `edit`, `archive`
and `unarchive`. Editing accepts any combination of title, body, explicit summary,
metadata object and replacement tag set. `--expected-revision` rejects stale
edits atomically. `tag add/remove`, `metadata set` and `summary set` are ordinary
versioned edits. Metadata set replaces the object; tags are case-sensitive,
deduplicated strings without whitespace. Archive never deletes content. An
archived forum hides its descendants from active views; an archived post hides
its comments. Restoring a parent reveals descendants unless they were separately
archived. Descendants keep their own archive flag and immutable history.
Explicit shows, history, references, `--archived` and `--all` retain access.

Every successful mutation creates an immutable complete revision and an activity
event in its transaction. Replies never modify the containing post's authored
body. Comments have their own revisions. `history ID --full` returns snapshots;
plain history is compact revision metadata, newest first.

Lists return summary and metadata while omitting bodies unless `--full`.
Comment lists and chronological discussions show full content by default.
Normal posts/forums lists sort by latest object edit then ID, newest first;
comments sort by creation then ID, oldest first. These are deterministic.
Use SQL for custom ordering/grouping. `--limit` and `--offset` paginate every
content list and an explicit `more` flag signals omitted rows.

Retrieval only prepares receipts. The output layer applies them after successfully
writing output. Compact/list receipts advance **seen**, complete current content
advances **read**, and truncated or omitted records do not advance either.
`updates` filters against seen revision, so scanning a title once suppresses its
unchanged repetition without claiming that its body has been read. It includes
changed-field names compared with the last read revision. `updates --full` uses
net patches for previously read objects and full content for new objects.
New comments appear as their own objects. `diff ID` computes a net diff from the last read revision to
current, skipping intermediate edits. If never read, its baseline is empty.
Explicit `--from`/`--to` selects historical comparisons. A diff advances read
only when it ends at current and begins at the known read baseline or at zero.

`thread POST` merges current post/comment content and generated activity into a
chronological timeline. `thread COMMENT` focuses on that comment and descendants;
`thread POST --tree` uses reply relationships. Generated entries remain separate
from authored content. Historical edits are available through history/diff.

Text references `#123` in title/body resolve to existing objects. Embedded tokens
such as `word#123`, `##123` and `#123suffix` do not resolve. Self-references and
nonexistent targets are left as text. Edits recompute outgoing links; additions
produce `reference.added` events on the target discussion. `links ID` returns
outgoing targets, `backlinks ID` returns current referring sources. References
never alter the task dependency DAG.

Module requests use dotted names, integer `id` targets (or a forum `path`),
`post` for comment creation/listing and `reply_to` for replies. Outputs contain
ordinary object items with `id`, `revision`, `kind`, and `body_omitted` when
applicable; diff output has both structured `changes` and plain unified `patch`.
