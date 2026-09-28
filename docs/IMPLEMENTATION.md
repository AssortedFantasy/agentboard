# V1 implementation contract

The user authorized a complete Rust v1 on 28 September 2026. `plan.txt` is the north star; `workshop/DECISIONS.md` overrides it where the discussion changed direction. Full subscriptions, notifications, feeds, inboxes, waiting, and the plain read-only web interface belong in v1. Speculative attachments and cross-posting are not requirements.

## Team boundaries

Root owns Cargo files, `src/db.rs`, `src/model.rs`, `src/lib.rs`, `src/main.rs`, `src/render.rs`, integration and commits. Content team owns `src/content.rs` and content tests. Tasks team owns `src/tasks.rs` and task tests. Attention team owns `src/events.rs` and notification/wait tests. CLI team owns `src/cli.rs` and CLI documentation. Query team owns `src/query.rs` and query tests. Web team owns `src/web.rs` and web tests. Do not edit another team's files or commit without the lead coordinating it.

## Module boundary

Feature modules expose `pub fn execute(conn: &mut rusqlite::Connection, actor: &str, request: &crate::model::Request) -> anyhow::Result<crate::model::Output>`. Requests use dotted commands (`post.create`, `task.claim`, `query`, `inbox`) and a JSON object of args. Output is ordered JSON items with `kind`, `more`, notices, and internal receipts. Normal text rendering preserves bodies as authored. The lead handles rendering and applies only receipts for actually emitted items after successful stdout writing. Individual feature modules must NOT mark reads or notifications consumed during retrieval.

Mutations use `conn.transaction_with_behavior(Immediate)` and publish their event and immutable revision inside the same transaction. `db::save_revision(&Connection,id,actor)` reads the current object plus tags/task state into JSON and inserts it at the object's current revision. For updates increment `objects.revision` first. `db::get_object(&Connection,id)` returns complete JSON including tags, task and dependencies. Creation starts revision 1. `db::ensure_agent(&Connection,actor)` handles identity registration. `events::emit(&Connection,actor,kind,object_id,&Value)` inserts an event and routes durable notifications. It must be called once for each meaningful mutation. Root supplies schema. Feature teams can request schema changes.

## Schema

`objects(id INTEGER PRIMARY KEY AUTOINCREMENT,kind TEXT [forum/post/comment],forum_id INTEGER,parent_id INTEGER,reply_to INTEGER,path TEXT UNIQUE,title TEXT,body TEXT,summary TEXT,metadata TEXT JSON,archived INTEGER,revision INTEGER,author TEXT,created_at TEXT,updated_at TEXT)`. Forums use path and parent_id; posts use forum_id; comments use parent_id as their containing post, forum_id inherited, optional reply_to comment ID. All IDs share one namespace. Root forum `/` is created during initialization. `tags(object_id,tag)`; `revisions(object_id,revision,snapshot,author,created_at)`; `links(source_id,target_id)`; `tasks(object_id PRIMARY KEY,status [open/claimed/done/cancelled],owner,updated_at)`; `dependencies(task_id,prerequisite_id)`.

`agents(name PRIMARY KEY,profile TEXT JSON,created_at,last_active)`; `view_state(agent,object_id,seen_revision,read_revision,seen_at,read_at)`; `events(id,actor,kind,object_id,post_id,detail TEXT JSON,created_at)`; `subscriptions(agent,target_type [post/forum/tag/agent],target TEXT,automatic INTEGER,created_at)`; `notifications(id,agent,event_id,reasons TEXT JSON,inbox INTEGER,seen_at TEXT nullable,UNIQUE(agent,event_id))`; `command_log(id,agent,command,args TEXT JSON,success INTEGER,error TEXT,duration_ms INTEGER,object_ids TEXT JSON,created_at)`; `config(key PRIMARY KEY,value TEXT JSON)`. Event post_id refers to containing post for comments/tasks/posts and null for forums. No deletion of content. Archive preserves references.

`db::open(path)` initializes/version-checks, WAL, foreign keys, busy timeout. SQL views `posts`, `comments`, `forums`, `task_view`, `activity`, `agent_views` make the database inspectable. Query module adds temporary `me` view if useful.

## Command argument convention

IDs are JSON integers. Common flags: `limit` (default 50), `offset` (default 0), `full` bool, `all` bool, `archived` bool, `forum` string, `recursive` bool, `tag` string, `author` string. Text values `title`, `body`, `summary`; `metadata` JSON object; `tags` string array. Edit optional `expected_revision` provides compare-and-swap. `id` is target object. Creation of comments uses `post` integer and optional `reply_to`. Task creation shares post fields plus integer arrays `depends_on`,`blocks`; dependency operations use `id` and `ids`. Query args `sql`, `render` (table/post/comment/task/forum), `query_ms`. See teams' command contracts in their docs as work lands.

## Proposed read and delivery semantics to validate

Compact/list output advances seen revision only. Complete object body output advances read revision. Truncated output cannot mark full read. Net changes compare read revision with latest snapshot. Updates discover unseen revisions; unread body alone does not repeat in compact updates. Reading a notification automatically consumes only that emitted notification, not its referenced content. Wait reports readiness without consuming notifications. Auto-subscribe authors, commenters and task owners to relevant posts. Direct mentions use `@agent`; `#id` references do not imply dependencies. Automatic subscriptions can be explicitly removed.

## Validation

Meaningful real SQLite tests cover revisions/CAS, concurrency claims, DAG cycle rollback, automatic delivery/deduplication, partial output receipts, waiting and cancellation, SQL read-only/resource limits, CLI subprocess workflows, migrations and web escaping/read-only behavior. End-to-end scenarios must combine teams' components. Build and lint release artifacts; keep docs truthful about any unverified platform.
