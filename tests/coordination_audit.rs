//! Cross-module workflows using the same dispatcher and receipt path as the executable.
use agentboard::{
    app, db,
    model::{Output, Request},
    render,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{path::Path, process::Command, time::Duration};

fn command(conn: &mut Connection, actor: &str, name: &str, args: Value) -> Output {
    app::execute(
        conn,
        actor,
        &Request {
            command: name.into(),
            args,
        },
    )
    .unwrap()
}

fn task(conn: &mut Connection, title: &str, args: Value) -> i64 {
    let mut args = args;
    args["title"] = json!(title);
    args["body"] = json!("Instructions whose delivery must be tracked separately.");
    command(conn, "lead", "task.create", args).items[0]["id"]
        .as_i64()
        .unwrap()
}

fn delivered(conn: &mut Connection, actor: &str, output: &Output) {
    let rendered = render::render(output, "json", false, 65536).unwrap();
    app::acknowledge(conn, actor, &rendered).unwrap();
}

#[test]
fn assignment_notification_and_content_receipts_remain_independent() {
    let mut conn = db::open(":memory:").unwrap();
    let id = task(&mut conn, "Assigned work", json!({"owner":"worker"}));
    let notification = command(&mut conn, "worker", "inbox", json!({}));
    assert_eq!(notification.items.len(), 1);
    assert!(
        notification.items[0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("assignment"))
    );
    delivered(&mut conn, "worker", &notification);
    assert!(
        command(&mut conn, "worker", "inbox", json!({}))
            .items
            .is_empty()
    );
    let state = command(&mut conn, "worker", "state.show", json!({"id":id}));
    assert!(
        state.items.is_empty(),
        "reading an assignment must not imply reading its instructions"
    );
    let post = command(&mut conn, "worker", "task.show", json!({"id":id}));
    delivered(&mut conn, "worker", &post);
    command(
        &mut conn,
        "lead",
        "comment.create",
        json!({"post":id,"body":"@worker please confirm this changed requirement"}),
    );
    let inbox = command(&mut conn, "worker", "inbox", json!({}));
    assert_eq!(
        inbox.items.len(),
        1,
        "mention plus owner auto-subscription is one event delivery"
    );
    command(&mut conn, "replacement", "task.takeover", json!({"id":id}));
    let before = command(&mut conn, "worker", "inbox", json!({}));
    assert!(
        !before.items.is_empty(),
        "takeover must not discard earlier pending work notifications"
    );
    let diff = command(&mut conn, "worker", "diff", json!({"id":id}));
    assert_eq!(
        diff.items[0]["changes"]["fields"]["task"]["before"]["owner"],
        "worker"
    );
    assert_eq!(
        diff.items[0]["changes"]["fields"]["task"]["after"]["owner"],
        "replacement"
    );
}

#[test]
fn live_dependency_wait_ignores_intermediate_completion_and_returns_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let mut conn = db::open(&path).unwrap();
    let first = task(&mut conn, "First prerequisite", json!({}));
    let second = task(&mut conn, "Second prerequisite", json!({}));
    let dependent = task(
        &mut conn,
        "Assigned dependent",
        json!({"owner":"worker","depends_on":[first,second]}),
    );
    let assignment = command(&mut conn, "worker", "inbox", json!({}));
    delivered(&mut conn, "worker", &assignment);
    let (sent, received) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut conn = db::open(path).unwrap();
        sent.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        command(&mut conn, "lead", "task.done", json!({"id":first}));
        std::thread::sleep(Duration::from_millis(100));
        command(&mut conn, "lead", "task.cancel", json!({"id":second}));
    });
    received.recv().unwrap();
    let wake = command(
        &mut conn,
        "worker",
        "wait",
        json!({"dependencies":dependent,"timeout":3,"poll_ms":10}),
    );
    writer.join().unwrap();
    assert_eq!(
        wake.items[0]["reasons"][0]["reason"],
        "dependency_cancelled"
    );
    assert_eq!(wake.items[0]["reasons"][0]["cancelled"], json!([second]));
    let pending = command(&mut conn, "worker", "inbox", json!({}));
    assert_eq!(
        pending.items.len(),
        2,
        "waiting must not consume either dependency notification"
    );
    let page = command(&mut conn, "worker", "inbox", json!({"limit":1}));
    delivered(&mut conn, "worker", &page);
    assert_eq!(
        command(&mut conn, "worker", "inbox", json!({})).items.len(),
        1
    );
    assert_eq!(
        command(&mut conn, "worker", "task.show", json!({"id":dependent})).items[0]["task"]["status"],
        "claimed"
    );
}

fn cli(path: &Path, actor: &str, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_agentboard"))
        .arg(path)
        .arg(actor)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn executable_coordinates_assignment_history_and_dependency_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cli.db");
    let before = cli(
        &path,
        "lead",
        &["task", "create", "/", "--title", "Prerequisite"],
    )["items"][0]["id"]
        .as_i64()
        .unwrap()
        .to_string();
    let after = cli(
        &path,
        "lead",
        &[
            "task",
            "create",
            "/",
            "--title",
            "Dependent",
            "--owner",
            "worker",
            "--depends-on",
            &before,
        ],
    )["items"][0]["id"]
        .as_i64()
        .unwrap()
        .to_string();
    assert_eq!(
        cli(
            &path,
            "worker",
            &["wait", "--dependencies", &after, "--timeout", "0"]
        )["items"][0]["ready"],
        false
    );
    cli(&path, "lead", &["task", "done", &before]);
    assert_eq!(
        cli(
            &path,
            "worker",
            &["wait", "--dependencies", &after, "--timeout", "0"]
        )["items"][0]["reasons"][0]["reason"],
        "dependencies_ready"
    );
    let inbox = cli(&path, "worker", &["inbox"]);
    assert!(
        inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["kind"] == "task.done")
    );
    cli(&path, "worker", &["task", "done", &after]);
    let timeline = cli(&path, "observer", &["thread", &after]);
    assert!(
        timeline["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["event_kind"] == "task.done")
    );
    let history = cli(&path, "observer", &["history", &after, "--full"]);
    assert_eq!(history["items"][0]["snapshot"]["task"]["status"], "done");
    assert_eq!(history["items"][1]["snapshot"]["task"]["status"], "claimed");
    assert!(
        cli(&path, "worker", &["inbox"])["items"]
            .as_array()
            .unwrap()
            .is_empty(),
        "self-completion does not create inbox noise"
    );
}

#[test]
fn task_archive_filter_selects_only_archived_work() {
    let mut conn = db::open(":memory:").unwrap();
    task(&mut conn, "Active", json!({}));
    let archived = task(&mut conn, "Archived", json!({}));
    command(&mut conn, "lead", "post.archive", json!({"id":archived}));
    let list = command(&mut conn, "worker", "task.list", json!({"archived":true}));
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0]["id"], archived);
}

#[test]
fn archived_ancestor_hides_work_and_restoring_it_recovers_readiness() {
    let mut conn = db::open(":memory:").unwrap();
    command(
        &mut conn,
        "lead",
        "forum.create",
        json!({"path":"/project"}),
    );
    command(
        &mut conn,
        "lead",
        "forum.create",
        json!({"path":"/project/child"}),
    );
    let id = task(&mut conn, "Nested work", json!({"forum":"/project/child"}));
    command(
        &mut conn,
        "lead",
        "forum.archive",
        json!({"path":"/project"}),
    );
    assert!(
        command(&mut conn, "worker", "task.list", json!({}))
            .items
            .is_empty()
    );
    assert!(
        command(&mut conn, "worker", "task.ready", json!({"all":true}))
            .items
            .is_empty()
    );
    assert_eq!(
        command(&mut conn, "worker", "task.show", json!({"id":id})).items[0]["ready"],
        false
    );
    assert!(
        app::execute(
            &mut conn,
            "worker",
            &Request {
                command: "task.claim".into(),
                args: json!({"id":id})
            }
        )
        .is_err()
    );
    assert_eq!(
        command(
            &mut conn,
            "worker",
            "wait",
            json!({"task_ready":true,"timeout":0})
        )
        .items[0]["ready"],
        false
    );
    assert_eq!(
        command(
            &mut conn,
            "worker",
            "wait",
            json!({"forum":"/","timeout":0})
        )
        .items[0]["ready"],
        false
    );
    assert_eq!(
        command(&mut conn, "worker", "task.list", json!({"archived":true})).items[0]["id"],
        id
    );
    command(
        &mut conn,
        "lead",
        "forum.unarchive",
        json!({"path":"/project"}),
    );
    assert_eq!(
        command(&mut conn, "worker", "task.ready", json!({})).items[0]["id"],
        id
    );
}

#[test]
fn assigning_does_not_fabricate_recipient_activity() {
    let mut conn = db::open(":memory:").unwrap();
    db::ensure_agent(&conn, "sleeping").unwrap();
    conn.execute(
        "UPDATE agents SET last_active='2000-01-01T00:00:00Z' WHERE name='sleeping'",
        [],
    )
    .unwrap();
    let id = task(&mut conn, "Future work", json!({"owner":"sleeping"}));
    assert_eq!(
        command(&mut conn, "lead", "agent.show", json!({"name":"sleeping"})).items[0]["last_active"],
        "2000-01-01T00:00:00Z"
    );
    command(&mut conn, "lead", "task.takeover", json!({"id":id}));
    command(
        &mut conn,
        "lead",
        "task.assign",
        json!({"id":id,"owner":"sleeping"}),
    );
    assert_eq!(
        command(&mut conn, "lead", "agent.show", json!({"name":"sleeping"})).items[0]["last_active"],
        "2000-01-01T00:00:00Z"
    );
}

#[test]
fn explicit_takeover_notifies_previous_owner_even_after_unsubscribing() {
    let mut conn = db::open(":memory:").unwrap();
    let id = task(&mut conn, "Previously owned", json!({"owner":"worker"}));
    let assignment = command(&mut conn, "worker", "inbox", json!({}));
    delivered(&mut conn, "worker", &assignment);
    command(
        &mut conn,
        "worker",
        "unsubscribe",
        json!({"target_type":"post","target":id}),
    );
    command(&mut conn, "replacement", "task.takeover", json!({"id":id}));
    let wake = command(
        &mut conn,
        "worker",
        "wait",
        json!({"inbox":true,"timeout":0}),
    );
    assert_eq!(wake.items[0]["ready"], true);
    let inbox = command(&mut conn, "worker", "inbox", json!({}));
    assert_eq!(inbox.items.len(), 1);
    assert!(
        inbox.items[0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("ownership_changed"))
    );
    assert_eq!(inbox.items[0]["detail"]["owner"], "replacement");
}
