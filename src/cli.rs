//! The CLI translates familiar commands into the shared execution contract.
use std::{ffi::OsString, io::Read, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};

use crate::model::Request;

#[derive(Debug)]
pub struct Invocation {
    pub database: PathBuf,
    pub actor: String,
    pub request: Request,
    pub format: String,
    pub compact: bool,
    pub max_bytes: usize,
}

#[derive(Debug, Parser)]
#[command(
    name = "agentboard",
    version,
    about = "A durable local collaboration board for agents",
    after_help = "Examples:\n  agentboard board.db alice post create / --title \"Plan\" --file plan.md\n  agentboard board.db bob task ready\n  agentboard board.db bob wait --inbox --timeout 60\n  agentboard board.db alice query 'SELECT id FROM posts' --render post\n  agentboard board.db serve\n\nUse `agentboard <command> --help` for command help without a database."
)]
pub struct Cli {
    /// SQLite database path (created automatically on first command).
    database: PathBuf,
    /// Stable caller identity; no authentication or harness integration required.
    actor: String,
    #[command(subcommand)]
    command: Command,
    /// Output a JSON envelope with items and omission metadata.
    #[arg(long, global = true, conflicts_with = "jsonl")]
    json: bool,
    /// Output one JSON item per line, followed by a metadata record.
    #[arg(long, global = true)]
    jsonl: bool,
    /// Show brief object records without full bodies.
    #[arg(short, long, global = true, conflicts_with = "full")]
    compact: bool,
    /// Include full content, subject to the total output budget.
    #[arg(long, global = true)]
    full: bool,
    /// Maximum returned objects/rows (default 50; maximum 100000).
    #[arg(long, global = true, value_parser = clap::value_parser!(u32).range(1..=100_000))]
    limit: Option<u32>,
    /// Skip this many matching objects/rows.
    #[arg(long, global = true, default_value_t = 0)]
    offset: u64,
    /// Total UTF-8 output budget in bytes (default 65536).
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1024..))]
    max_bytes: Option<u64>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a board and display its schema version.
    Init,
    /// Show the documented SQLite schema and query views.
    Schema,
    /// Create and organize hierarchical forums.
    Forum {
        #[command(subcommand)]
        command: ForumCommand,
    },
    /// Publish, read, edit and archive posts.
    Post {
        #[command(subcommand)]
        command: PostCommand,
    },
    /// Add comments or replies to a post.
    Comment {
        #[command(subcommand)]
        command: CommentCommand,
    },
    /// Coordinate work with atomic ownership and explicit dependencies.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Read a chronological discussion or a focused reply tree.
    Thread(ThreadArgs),
    /// Inspect immutable revisions, newest first.
    History(IdArgs),
    /// Show a net diff from your last full read, or explicit revisions.
    Diff(DiffArgs),
    /// Discover objects with revisions you have not yet seen.
    Updates(UpdatesArgs),
    /// List outgoing #id references.
    Links(IdArgs),
    /// List objects that reference this object.
    Backlinks(IdArgs),
    /// Add/remove tags without rewriting content.
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
    /// Replace an object's structured metadata.
    Metadata {
        #[command(subcommand)]
        command: MetadataCommand,
    },
    /// Maintain an explicit summary independently of the body.
    Summary {
        #[command(subcommand)]
        command: SummaryCommand,
    },
    /// Search titles and bodies using SQLite FTS5 expressions.
    Search(SearchArgs),
    /// Run read-only SQLite SQL; optionally render selected object IDs.
    Query(QueryArgs),
    /// Subscribe to future activity on a post, forum, tag or agent.
    Subscribe(SubscriptionArgs),
    /// Remove an explicit or automatic subscription.
    Unsubscribe(SubscriptionArgs),
    /// Inspect your subscriptions.
    Subscriptions(SubscriptionsArgs),
    /// Retrieve unseen subscribed activity; emitted notifications are consumed.
    Feed(FeedArgs),
    /// Retrieve unseen direct attention events (mentions, replies, assignments).
    Inbox(FeedArgs),
    /// Inspect the board-wide event log.
    Activity(ActivityArgs),
    /// Wait until any selected condition is ready; consumes nothing.
    Wait(WaitArgs),
    /// Register identities and maintain lightweight profiles.
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Inspect or reset your persisted observation state.
    State {
        #[command(subcommand)]
        command: StateCommand,
    },
    /// Inspect recent CLI operations and failures.
    Log(LogArgs),
    /// Inspect or change persistent board configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Serve the plain read-only web interface (Ctrl-C to stop).
    Serve(ServeArgs),
}

#[derive(Debug, Args, Serialize, Default)]
struct BodyArgs {
    /// Literal UTF-8 body. For longer text, use --file or --stdin.
    #[arg(long, conflicts_with_all = ["file", "stdin"])]
    body: Option<String>,
    /// Read the body from a UTF-8 file; '-' means stdin.
    #[arg(long, conflicts_with = "stdin")]
    file: Option<PathBuf>,
    /// Read the body from stdin without shell-escaping its contents.
    #[arg(long)]
    stdin: bool,
}

#[derive(Debug, Args, Serialize, Default)]
struct Fields {
    #[command(flatten)]
    #[serde(flatten)]
    input: BodyArgs,
    #[arg(long)]
    summary: Option<String>,
    /// Comma-separated or repeated tags.
    #[arg(long = "tag", value_delimiter = ',')]
    tags: Vec<String>,
    /// JSON object; replaces metadata on edit.
    #[arg(long, value_parser = parse_object)]
    metadata: Option<Value>,
}

#[derive(Debug, Args, Serialize)]
struct NewPost {
    /// Destination forum path.
    #[arg(default_value = "/")]
    forum: String,
    #[arg(long)]
    title: String,
    #[command(flatten)]
    #[serde(flatten)]
    fields: Fields,
}

#[derive(Debug, Args, Serialize)]
struct NewForum {
    /// Absolute hierarchical path, for example /compiler/testing.
    path: String,
    #[arg(long)]
    title: Option<String>,
    #[command(flatten)]
    #[serde(flatten)]
    fields: Fields,
}

#[derive(Debug, Args, Serialize)]
struct EditArgs {
    id: i64,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    expected_revision: Option<i64>,
    #[command(flatten)]
    #[serde(flatten)]
    fields: Fields,
}

#[derive(Debug, Args, Serialize)]
struct ForumEdit {
    /// Forum path or ID.
    target: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    expected_revision: Option<i64>,
    #[command(flatten)]
    #[serde(flatten)]
    fields: Fields,
}

#[derive(Debug, Args, Serialize)]
struct IdArgs {
    id: i64,
}

#[derive(Debug, Args, Serialize)]
struct MutationArgs {
    id: i64,
    /// Fail if another writer has changed this revision.
    #[arg(long)]
    expected_revision: Option<i64>,
}

#[derive(Debug, Args, Serialize)]
struct ForumTarget {
    /// Forum path or ID.
    target: String,
    #[arg(long)]
    expected_revision: Option<i64>,
}

#[derive(Debug, Args, Serialize)]
struct ShowArgs {
    id: i64,
    /// Show a net diff since your last full read.
    #[arg(long)]
    changes: bool,
    /// Show a net diff from this explicit revision.
    #[arg(long)]
    since_version: Option<i64>,
}

#[derive(Debug, Args, Serialize, Default)]
struct ListArgs {
    #[arg(long)]
    forum: Option<String>,
    #[arg(short, long)]
    recursive: bool,
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    author: Option<String>,
    /// Include active and archived objects.
    #[arg(long, conflicts_with = "archived")]
    all: bool,
    /// Return only archived objects.
    #[arg(long)]
    archived: bool,
}

#[derive(Debug, Args, Serialize)]
struct UpdatesArgs {
    #[command(flatten)]
    #[serde(flatten)]
    filters: ListArgs,
    #[arg(long, value_parser = ["post", "comment", "forum", "task"])]
    kind: Option<String>,
}

#[derive(Debug, Subcommand)]
enum ForumCommand {
    Create(NewForum),
    Show(ForumTarget),
    List(ListArgs),
    Edit(ForumEdit),
    Archive(ForumTarget),
    Unarchive(ForumTarget),
}

#[derive(Debug, Subcommand)]
enum PostCommand {
    Create(NewPost),
    Show(ShowArgs),
    List(ListArgs),
    Edit(EditArgs),
    Archive(MutationArgs),
    Unarchive(MutationArgs),
}

#[derive(Debug, Args, Serialize)]
struct NewComment {
    /// Containing post ID.
    post: i64,
    #[arg(long)]
    reply_to: Option<i64>,
    #[command(flatten)]
    #[serde(flatten)]
    fields: Fields,
}

#[derive(Debug, Args, Serialize)]
struct CommentList {
    post: i64,
    #[arg(long, conflicts_with = "archived")]
    all: bool,
    #[arg(long)]
    archived: bool,
}

#[derive(Debug, Subcommand)]
enum CommentCommand {
    Create(NewComment),
    Show(ShowArgs),
    List(CommentList),
    Edit(EditArgs),
    Archive(MutationArgs),
    Unarchive(MutationArgs),
}

#[derive(Debug, Args, Serialize)]
struct TaskRelations {
    /// Prerequisite task IDs; space-separated or comma-separated.
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    depends_on: Vec<i64>,
    /// Task IDs for which this task is a prerequisite.
    #[arg(long, num_args = 1.., value_delimiter = ',')]
    blocks: Vec<i64>,
    #[arg(long)]
    owner: Option<String>,
}

#[derive(Debug, Args, Serialize)]
struct NewTask {
    #[command(flatten)]
    #[serde(flatten)]
    post: NewPost,
    #[command(flatten)]
    #[serde(flatten)]
    relations: TaskRelations,
}

#[derive(Debug, Args, Serialize)]
struct AttachTask {
    #[command(flatten)]
    #[serde(flatten)]
    target: MutationArgs,
    #[command(flatten)]
    #[serde(flatten)]
    relations: TaskRelations,
}

#[derive(Debug, Args, Serialize)]
struct TaskList {
    #[command(flatten)]
    #[serde(flatten)]
    filters: ListArgs,
    #[arg(long, value_parser = ["open", "claimed", "done", "cancelled"])]
    status: Option<String>,
    #[arg(long)]
    owner: Option<String>,
}

#[derive(Debug, Args, Serialize)]
struct AssignTask {
    #[command(flatten)]
    #[serde(flatten)]
    target: MutationArgs,
    #[arg(long)]
    owner: String,
    /// Explicitly replace another agent's ownership.
    #[arg(long)]
    takeover: bool,
}

#[derive(Debug, Args, Serialize)]
struct TaskMutationArgs {
    #[command(flatten)]
    #[serde(flatten)]
    target: MutationArgs,
    /// Explicitly override another agent's ownership.
    #[arg(long)]
    takeover: bool,
}

#[derive(Debug, Args, Serialize)]
struct TakeoverTask {
    #[command(flatten)]
    #[serde(flatten)]
    target: MutationArgs,
    /// New owner (defaults to your identity).
    #[arg(long)]
    owner: Option<String>,
}

#[derive(Debug, Args, Serialize)]
struct DependencyArgs {
    #[command(flatten)]
    #[serde(flatten)]
    target: MutationArgs,
    /// Other task IDs; this entire change is atomic.
    #[arg(required = true, num_args = 1.., value_delimiter = ',')]
    ids: Vec<i64>,
    /// Remove these edges instead of adding them.
    #[arg(long)]
    remove: bool,
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    Create(NewTask),
    Attach(AttachTask),
    Show(ShowArgs),
    List(TaskList),
    Ready(TaskList),
    Claim(MutationArgs),
    Release(TaskMutationArgs),
    Assign(AssignTask),
    Takeover(TakeoverTask),
    Done(TaskMutationArgs),
    Cancel(TaskMutationArgs),
    Reopen(MutationArgs),
    /// This task depends on the following task IDs.
    Depend(DependencyArgs),
    /// This task blocks the following task IDs.
    Block(DependencyArgs),
    Edit(EditArgs),
    Archive(MutationArgs),
    Unarchive(MutationArgs),
}

#[derive(Debug, Args, Serialize)]
struct ThreadArgs {
    /// Post ID for the complete discussion, or comment ID for its subtree.
    id: i64,
    /// Render reply relationships in tree order.
    #[arg(long)]
    tree: bool,
}

#[derive(Debug, Args, Serialize)]
struct DiffArgs {
    id: i64,
    #[arg(long)]
    from: Option<i64>,
    #[arg(long)]
    to: Option<i64>,
}

#[derive(Debug, Args, Serialize)]
struct TagsArgs {
    id: i64,
    #[arg(required = true, num_args = 1.., value_delimiter = ',')]
    tags: Vec<String>,
    #[arg(long)]
    expected_revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
enum TagCommand {
    Add(TagsArgs),
    Remove(TagsArgs),
    List,
}

#[derive(Debug, Args, Serialize)]
struct MetadataArgs {
    id: i64,
    /// Complete metadata JSON object.
    #[arg(value_parser = parse_object)]
    metadata: Value,
    #[arg(long)]
    expected_revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
enum MetadataCommand {
    Set(MetadataArgs),
}

#[derive(Debug, Args, Serialize)]
struct SummaryArgs {
    id: i64,
    /// New summary; alternatively use --file or --stdin.
    #[arg(conflicts_with_all = ["file", "stdin"])]
    summary: Option<String>,
    #[arg(long, conflicts_with = "stdin")]
    file: Option<PathBuf>,
    #[arg(long)]
    stdin: bool,
    #[arg(long)]
    expected_revision: Option<i64>,
}

#[derive(Debug, Subcommand)]
enum SummaryCommand {
    Set(SummaryArgs),
}

#[derive(Debug, Args, Serialize)]
struct SearchArgs {
    /// FTS5 expression: words, "phrases", AND/OR/NOT, prefix*.
    text: String,
    #[command(flatten)]
    #[serde(flatten)]
    filters: ListArgs,
    #[arg(long, value_parser = ["post", "comment", "forum", "task"])]
    kind: Option<String>,
    /// SQLite execution budget in milliseconds (default 1000).
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=300_000))]
    query_ms: Option<u64>,
}

#[derive(Debug, Clone, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
enum Render {
    Table,
    Post,
    Comment,
    Task,
    Forum,
}

#[derive(Debug, Args, Serialize)]
struct QueryArgs {
    /// One read-only SQL statement, or use --file / --stdin.
    #[arg(conflicts_with_all = ["file", "stdin"])]
    sql: Option<String>,
    #[arg(long, conflicts_with = "stdin")]
    file: Option<PathBuf>,
    #[arg(long)]
    stdin: bool,
    /// Object rendering requires exactly one result column named id.
    #[arg(long, value_enum, default_value = "table")]
    render: Render,
    /// SQLite execution budget in milliseconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=300_000))]
    query_ms: Option<u64>,
}

#[derive(Debug, Args, Serialize)]
struct SubscriptionArgs {
    #[arg(value_parser = ["post", "forum", "tag", "agent"])]
    target_type: String,
    /// Post ID, forum path/ID, tag or agent name.
    target: String,
    /// Also route matching subscription activity to your inbox.
    #[arg(long)]
    inbox: bool,
}

#[derive(Debug, Args, Serialize)]
struct SubscriptionsArgs {
    /// Include subscriptions explicitly disabled with unsubscribe.
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Args, Serialize)]
struct FeedArgs {
    /// Include notifications already consumed.
    #[arg(long)]
    all: bool,
    #[arg(long)]
    kind: Option<String>,
    /// Filter events by their originating agent.
    #[arg(long)]
    actor: Option<String>,
    /// Only events after this event ID.
    #[arg(long)]
    since: Option<i64>,
}

#[derive(Debug, Args, Serialize)]
struct ActivityArgs {
    #[command(flatten)]
    #[serde(flatten)]
    filters: FeedArgs,
    #[arg(long)]
    object: Option<i64>,
    #[arg(long)]
    post: Option<i64>,
}

#[derive(Debug, Args, Serialize)]
struct WaitArgs {
    #[arg(long)]
    inbox: bool,
    #[arg(long)]
    subscriptions: bool,
    #[arg(long)]
    task_ready: bool,
    /// Return when all prerequisites finish, or any is cancelled.
    #[arg(long)]
    dependencies: Option<i64>,
    #[arg(long)]
    post: Option<i64>,
    #[arg(long)]
    forum: Option<String>,
    /// Maximum wait in seconds; zero checks once.
    #[arg(long, default_value_t = 60)]
    timeout: u64,
    /// Internal polling interval in milliseconds.
    #[arg(long, value_parser = clap::value_parser!(u64).range(10..))]
    poll_ms: Option<u64>,
}

#[derive(Debug, Args, Serialize)]
struct AgentNew {
    /// Register this exact name; omitted generates a unique prefixed name.
    name: Option<String>,
    #[arg(long, default_value = "agent")]
    prefix: String,
}

#[derive(Debug, Args, Serialize)]
struct AgentShow {
    name: Option<String>,
}

#[derive(Debug, Args, Serialize)]
struct AgentProfile {
    /// Profile JSON object; omit to show the current profile.
    #[arg(long, value_parser = parse_object)]
    metadata: Option<Value>,
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    New(AgentNew),
    List,
    Show(AgentShow),
    Profile(AgentProfile),
}

#[derive(Debug, Args, Serialize)]
struct StateArgs {
    /// Restrict to one object; omitted addresses your whole view state.
    id: Option<i64>,
}

#[derive(Debug, Subcommand)]
enum StateCommand {
    Show(StateArgs),
    Reset(StateArgs),
}

#[derive(Debug, Args, Serialize)]
struct LogArgs {
    #[arg(long, visible_alias = "agent")]
    author: Option<String>,
    #[arg(long)]
    command: Option<String>,
    #[arg(long)]
    failed: bool,
}

#[derive(Debug, Args, Serialize)]
struct ConfigGet {
    key: String,
}

#[derive(Debug, Args, Serialize)]
struct ConfigSet {
    key: String,
    /// JSON value, or an unquoted string.
    value: String,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Get(ConfigGet),
    Set(ConfigSet),
    List,
}

#[derive(Debug, Args, Serialize)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,
}

fn parse_object(value: &str) -> std::result::Result<Value, String> {
    let parsed: Value = serde_json::from_str(value).map_err(|e| format!("invalid JSON: {e}"))?;
    if !parsed.is_object() {
        return Err("expected a JSON object, for example {\"priority\":\"high\"}".into());
    }
    Ok(parsed)
}

fn request(command: &str, args: impl Serialize) -> Result<Request> {
    let mut args = serde_json::to_value(args)?;
    if args.is_null() {
        args = json!({});
    }
    if let Some(map) = args.as_object_mut() {
        map.retain(|_, value| !value.is_null());
        // An omitted --tag does not clear existing tags on an edit.
        if map
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            map.remove("tags");
        }
        if let Some(target) = map.remove("target") {
            if command.starts_with("forum.") {
                let target = target.as_str().unwrap_or_default();
                if let Ok(id) = target.parse::<i64>() {
                    map.insert("id".into(), json!(id));
                } else {
                    map.insert("path".into(), json!(target));
                }
            } else {
                map.insert("target".into(), target);
            }
        }
        let file = map
            .remove("file")
            .and_then(|v| v.as_str().map(str::to_owned));
        let stdin = map
            .remove("stdin")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let text_key = if command == "summary.set" {
            "summary"
        } else if command == "query" {
            "sql"
        } else {
            "body"
        };
        if stdin || file.as_deref() == Some("-") {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .context("could not read UTF-8 text from stdin")?;
            map.insert(text_key.into(), json!(text));
        } else if let Some(path) = file {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("could not read UTF-8 file {path}"))?;
            map.insert(text_key.into(), json!(text));
        }
    }
    if command == "query" && args.get("sql").is_none() {
        bail!("query requires SQL text, --file PATH, or --stdin");
    }
    if command == "summary.set" && args.get("summary").is_none() {
        bail!("summary set requires text, --file PATH, or --stdin (use an empty string to clear)");
    }
    if command == "comment.create" && args.get("body").is_none() {
        bail!("comment create requires --body TEXT, --file PATH, or --stdin");
    }
    Ok(Request {
        command: command.into(),
        args,
    })
}

fn show(command: &str, args: ShowArgs) -> Result<Request> {
    if args.changes || args.since_version.is_some() {
        request("diff", json!({"id":args.id,"from":args.since_version}))
    } else {
        request(command, json!({"id":args.id}))
    }
}

impl Cli {
    pub fn into_invocation(self) -> Result<Invocation> {
        if self.actor.trim().is_empty() {
            bail!("agent identity cannot be empty");
        }
        let mut req = match self.command {
            Command::Init => request("init", ())?,
            Command::Schema => request("schema", ())?,
            Command::Forum { command } => match command {
                ForumCommand::Create(a) => request("forum.create", a)?,
                ForumCommand::Show(a) => request("forum.show", a)?,
                ForumCommand::List(a) => request("forum.list", a)?,
                ForumCommand::Edit(a) => request("forum.edit", a)?,
                ForumCommand::Archive(a) => request("forum.archive", a)?,
                ForumCommand::Unarchive(a) => request("forum.unarchive", a)?,
            },
            Command::Post { command } => match command {
                PostCommand::Create(a) => request("post.create", a)?,
                PostCommand::Show(a) => show("post.show", a)?,
                PostCommand::List(a) => request("post.list", a)?,
                PostCommand::Edit(a) => request("post.edit", a)?,
                PostCommand::Archive(a) => request("post.archive", a)?,
                PostCommand::Unarchive(a) => request("post.unarchive", a)?,
            },
            Command::Comment { command } => match command {
                CommentCommand::Create(a) => request("comment.create", a)?,
                CommentCommand::Show(a) => show("comment.show", a)?,
                CommentCommand::List(a) => request("comment.list", a)?,
                CommentCommand::Edit(a) => request("comment.edit", a)?,
                CommentCommand::Archive(a) => request("comment.archive", a)?,
                CommentCommand::Unarchive(a) => request("comment.unarchive", a)?,
            },
            Command::Task { command } => match command {
                TaskCommand::Create(a) => request("task.create", a)?,
                TaskCommand::Attach(a) => request("task.attach", a)?,
                TaskCommand::Show(a) => show("task.show", a)?,
                TaskCommand::List(a) => request("task.list", a)?,
                TaskCommand::Ready(a) => request("task.ready", a)?,
                TaskCommand::Claim(a) => request("task.claim", a)?,
                TaskCommand::Release(a) => request("task.release", a)?,
                TaskCommand::Assign(a) => request("task.assign", a)?,
                TaskCommand::Takeover(a) => request("task.takeover", a)?,
                TaskCommand::Done(a) => request("task.done", a)?,
                TaskCommand::Cancel(a) => request("task.cancel", a)?,
                TaskCommand::Reopen(a) => request("task.reopen", a)?,
                TaskCommand::Depend(a) => request("task.depend", a)?,
                TaskCommand::Block(a) => request("task.block", a)?,
                TaskCommand::Edit(a) => request("post.edit", a)?,
                TaskCommand::Archive(a) => request("post.archive", a)?,
                TaskCommand::Unarchive(a) => request("post.unarchive", a)?,
            },
            Command::Thread(a) => request("thread", a)?,
            Command::History(a) => request("history", a)?,
            Command::Diff(a) => request("diff", a)?,
            Command::Updates(a) => request("updates", a)?,
            Command::Links(a) => request("links", a)?,
            Command::Backlinks(a) => request("backlinks", a)?,
            Command::Tag { command } => match command {
                TagCommand::Add(a) => request("tag.add", a)?,
                TagCommand::Remove(a) => request("tag.remove", a)?,
                TagCommand::List => request("tag.list", ())?,
            },
            Command::Metadata {
                command: MetadataCommand::Set(a),
            } => request("metadata.set", a)?,
            Command::Summary {
                command: SummaryCommand::Set(a),
            } => request("summary.set", a)?,
            Command::Search(a) => request("search", a)?,
            Command::Query(a) => request("query", a)?,
            Command::Subscribe(a) => request("subscribe", a)?,
            Command::Unsubscribe(a) => request("unsubscribe", a)?,
            Command::Subscriptions(a) => request("subscriptions", a)?,
            Command::Feed(a) => request("feed", a)?,
            Command::Inbox(a) => request("inbox", a)?,
            Command::Activity(a) => request("activity", a)?,
            Command::Wait(a) => request("wait", a)?,
            Command::Agent { command } => match command {
                AgentCommand::New(a) => request("agent.new", a)?,
                AgentCommand::List => request("agent.list", ())?,
                AgentCommand::Show(a) => request("agent.show", a)?,
                AgentCommand::Profile(a) if a.metadata.is_none() => request("agent.show", ())?,
                AgentCommand::Profile(a) => request("agent.profile", a)?,
            },
            Command::State { command } => match command {
                StateCommand::Show(a) => request("state.show", a)?,
                StateCommand::Reset(a) => request("state.reset", a)?,
            },
            Command::Log(a) => request("log", a)?,
            Command::Config { command } => match command {
                ConfigCommand::Get(a) => request("config.get", a)?,
                ConfigCommand::Set(a) => {
                    let value = serde_json::from_str::<Value>(&a.value).unwrap_or(json!(a.value));
                    request("config.set", json!({"key":a.key,"value":value}))?
                }
                ConfigCommand::List => request("config.list", ())?,
            },
            Command::Serve(a) => request("serve", a)?,
        };
        req.args["limit"] = json!(self.limit.unwrap_or(50));
        req.args["offset"] = json!(self.offset);
        req.args["max_bytes"] = json!(self.max_bytes.unwrap_or(65_536));
        if self.full {
            req.args["full"] = json!(true);
        }
        if self.compact {
            req.args["compact"] = json!(true);
            req.args["full"] = json!(false);
        }
        req.args["_explicit_limit"] = json!(self.limit.is_some());
        req.args["_explicit_format"] = json!(self.json || self.jsonl);
        req.args["_explicit_max_bytes"] = json!(self.max_bytes.is_some());
        req.args["_explicit_compact"] = json!(self.compact || self.full);
        req.args["_explicit_query_ms"] = json!(req.args.get("query_ms").is_some());
        req.args["_explicit_poll_ms"] = json!(req.args.get("poll_ms").is_some());
        Ok(Invocation {
            database: self.database,
            actor: self.actor,
            request: req,
            format: if self.json {
                "json"
            } else if self.jsonl {
                "jsonl"
            } else {
                "text"
            }
            .into(),
            compact: self.compact,
            max_bytes: self
                .max_bytes
                .unwrap_or(65_536)
                .try_into()
                .context("output byte budget exceeds this platform's address space")?,
        })
    }
}

fn normalize(args: impl IntoIterator<Item = impl Into<OsString>>) -> Vec<OsString> {
    let mut args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let no_context_help = args.get(1).is_some_and(|a| a == "help")
        || (args.iter().skip(1).any(|a| a == "--help" || a == "-h")
            && args.get(1).is_some_and(|a| {
                matches!(
                    a.to_str(),
                    Some(
                        "forum"
                            | "post"
                            | "comment"
                            | "task"
                            | "thread"
                            | "history"
                            | "diff"
                            | "updates"
                            | "links"
                            | "backlinks"
                            | "tag"
                            | "metadata"
                            | "summary"
                            | "search"
                            | "query"
                            | "subscribe"
                            | "unsubscribe"
                            | "subscriptions"
                            | "feed"
                            | "inbox"
                            | "activity"
                            | "wait"
                            | "agent"
                            | "state"
                            | "log"
                            | "config"
                            | "serve"
                            | "schema"
                            | "init"
                    )
                )
            }));
    if no_context_help {
        args.insert(1, "<database>".into());
        args.insert(2, "<agent>".into());
    } else if args.get(2).is_some_and(|a| a == "serve")
        && args
            .get(3)
            .is_none_or(|a| a.to_string_lossy().starts_with('-'))
    {
        args.insert(2, "web-observer".into());
    }
    args
}

/// Parse process arguments. Clap prints help/version and exits successfully.
pub fn parse() -> Result<Invocation> {
    Cli::parse_from(normalize(std::env::args_os())).into_invocation()
}

/// Parse supplied arguments without exiting (useful to embedders and tests).
pub fn parse_from<I, T>(args: I) -> Result<Invocation>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    Cli::try_parse_from(normalize(args))?.into_invocation()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse_tail(args: &[&str]) -> Result<Invocation> {
        parse_from(
            ["agentboard", "board.db", "alice"]
                .into_iter()
                .chain(args.iter().copied()),
        )
    }
    #[test]
    fn parser_definition_is_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
    #[test]
    fn dependencies_and_global_options_survive_translation() {
        let inv = parse_tail(&[
            "task",
            "create",
            "/work",
            "--title",
            "Build",
            "--depends-on",
            "2",
            "3",
            "--blocks",
            "9,10",
            "--json",
            "--limit",
            "8",
        ])
        .unwrap();
        assert_eq!(inv.request.command, "task.create");
        assert_eq!(inv.request.args["depends_on"], json!([2, 3]));
        assert_eq!(inv.request.args["blocks"], json!([9, 10]));
        assert_eq!(inv.request.args["limit"], 8);
        assert_eq!(inv.format, "json");
    }
    #[test]
    fn conflicting_input_and_output_modes_are_rejected() {
        assert!(parse_tail(&["post", "create", "--title", "x", "--body", "x", "--stdin"]).is_err());
        assert!(parse_tail(&["post", "list", "--json", "--jsonl"]).is_err());
        assert!(parse_tail(&["post", "list", "--full", "--compact"]).is_err());
    }
    #[test]
    fn utf8_file_body_is_preserved_without_escaping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.md");
        let body = "# Hello\n雪 \\\"quoted\\\"\n";
        std::fs::write(&path, body).unwrap();
        let inv = parse_tail(&[
            "post",
            "create",
            "--title",
            "x",
            "--file",
            path.to_str().unwrap(),
        ])
        .unwrap();
        assert_eq!(inv.request.args["body"], body);
        assert!(inv.request.args.get("file").is_none());
    }
    #[test]
    fn metadata_requires_objects_and_edits_preserve_unspecified_tags() {
        assert!(parse_tail(&["post", "edit", "2", "--metadata", "[]"]).is_err());
        let inv = parse_tail(&["post", "edit", "2", "--body", "changed"]).unwrap();
        assert!(inv.request.args.get("tags").is_none());
    }
    #[test]
    fn changes_and_paths_translate_unambiguously() {
        assert_eq!(
            parse_tail(&["post", "show", "2", "--since-version", "3"])
                .unwrap()
                .request
                .args["from"],
            3
        );
        assert_eq!(
            parse_tail(&["forum", "show", "/work"])
                .unwrap()
                .request
                .args["path"],
            "/work"
        );
        assert_eq!(
            parse_tail(&["forum", "show", "7"]).unwrap().request.args["id"],
            7
        );
    }
    #[test]
    fn convenience_help_and_serve_parse() {
        let err = parse_from(["agentboard", "task", "--help"]).unwrap_err();
        assert!(err.to_string().contains("atomic ownership"));
        assert_eq!(
            parse_from(["agentboard", "board.db", "serve"])
                .unwrap()
                .request
                .command,
            "serve"
        );
    }
    #[test]
    fn documented_command_families_parse() {
        let examples: &[&[&str]] = &[
            &["init"],
            &["schema"],
            &[
                "forum",
                "create",
                "/compiler",
                "--title",
                "Compiler",
                "--body",
                "Compiler work",
            ],
            &["forum", "list", "--forum", "/", "--recursive"],
            &["forum", "show", "/compiler"],
            &[
                "forum",
                "edit",
                "/compiler",
                "--summary",
                "Current compiler work",
            ],
            &["forum", "archive", "/compiler"],
            &["forum", "unarchive", "/compiler"],
            &[
                "post",
                "create",
                "/compiler",
                "--title",
                "Parser design",
                "--tag",
                "decision",
            ],
            &[
                "post",
                "list",
                "--forum",
                "/compiler",
                "--recursive",
                "--tag",
                "decision",
                "--author",
                "alice",
            ],
            &["post", "show", "42"],
            &["post", "show", "42", "--changes"],
            &[
                "post",
                "edit",
                "42",
                "--title",
                "Updated",
                "--expected-revision",
                "2",
            ],
            &["post", "archive", "42"],
            &["post", "unarchive", "42"],
            &["comment", "create", "42", "--body", "See #51, @bob"],
            &[
                "comment",
                "create",
                "42",
                "--reply-to",
                "43",
                "--body",
                "Reply",
            ],
            &["comment", "list", "42"],
            &["comment", "show", "43"],
            &["comment", "edit", "43", "--body", "Updated comment"],
            &["comment", "archive", "43"],
            &["comment", "unarchive", "43"],
            &["thread", "42"],
            &["thread", "43", "--tree"],
            &["history", "42"],
            &["diff", "42"],
            &["diff", "42", "--from", "2", "--to", "5"],
            &["updates", "--forum", "/compiler", "--recursive"],
            &["links", "42"],
            &["backlinks", "42"],
            &["tag", "add", "42", "decision", "reviewed"],
            &["tag", "remove", "42", "reviewed"],
            &["tag", "list"],
            &["metadata", "set", "42", "{\"priority\":\"high\"}"],
            &["summary", "set", "42", "Summary"],
            &[
                "task",
                "create",
                "/compiler",
                "--title",
                "Parser",
                "--depends-on",
                "12",
                "13",
                "--blocks",
                "30",
            ],
            &[
                "task",
                "attach",
                "42",
                "--depends-on",
                "12",
                "--owner",
                "bob",
            ],
            &["task", "show", "42"],
            &["task", "list", "--status", "claimed", "--owner", "bob"],
            &["task", "ready"],
            &["task", "claim", "42"],
            &["task", "release", "42", "--takeover"],
            &["task", "assign", "42", "--owner", "bob"],
            &["task", "takeover", "42"],
            &["task", "done", "42"],
            &["task", "cancel", "42"],
            &["task", "reopen", "42"],
            &["task", "depend", "42", "12", "13"],
            &["task", "block", "12", "42", "43"],
            &["task", "depend", "42", "13", "--remove"],
            &["subscribe", "forum", "/compiler"],
            &["subscribe", "post", "42"],
            &["subscribe", "tag", "decision", "--inbox"],
            &["subscribe", "agent", "bob"],
            &["subscriptions"],
            &["subscriptions", "--all"],
            &["unsubscribe", "post", "42"],
            &["feed"],
            &["inbox"],
            &["feed", "--all", "--since", "100"],
            &["activity", "--post", "42"],
            &["activity", "--actor", "bob", "--kind", "task.done"],
            &["wait", "--inbox", "--timeout", "60"],
            &[
                "wait",
                "--subscriptions",
                "--task-ready",
                "--timeout",
                "120",
            ],
            &["wait", "--dependencies", "42", "--timeout", "120"],
            &["wait", "--post", "42", "--timeout", "60"],
            &["wait", "--forum", "/compiler", "--timeout", "60"],
            &[
                "search",
                "parser AND unicode",
                "--forum",
                "/compiler",
                "--recursive",
                "--query-ms",
                "2000",
            ],
            &[
                "query",
                "SELECT id FROM posts ORDER BY id",
                "--render",
                "post",
            ],
            &["agent", "new", "--prefix", "compiler-worker"],
            &["agent", "new", "bob"],
            &["agent", "list"],
            &["agent", "show", "bob"],
            &["agent", "profile"],
            &["agent", "profile", "--metadata", "{\"role\":\"parser\"}"],
            &["state", "show"],
            &["state", "reset", "42"],
            &["log", "--author", "bob"],
            &["log", "--failed"],
            &["config", "list"],
            &["config", "get", "limit"],
            &["config", "set", "limit", "100"],
            &["serve", "--bind", "127.0.0.1:8080"],
        ];
        for args in examples {
            parse_tail(args).unwrap_or_else(|e| panic!("{args:?}: {e:#}"));
        }
    }
}
