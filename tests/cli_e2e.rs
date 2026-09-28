use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(db: &Path, actor: &str, args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agentboard"));
    command
        .arg(db)
        .arg(actor)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn json_run(db: &Path, actor: &str, args: &[&str]) -> Value {
    let mut args = args.to_vec();
    args.push("--json");
    let output = run(db, actor, &args, None);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn first_id(value: &Value) -> String {
    value["items"][0]["id"].as_i64().unwrap().to_string()
}

#[test]
fn content_roundtrip_net_diff_and_observation_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("board.db");
    json_run(&db, "alice", &["init"]);
    json_run(&db, "alice", &["forum", "create", "/work"]);
    let path = dir.path().join("body.md");
    let body = "# Draft\nUnicode 雪 and \"quotes\" with C:\\work\n";
    std::fs::write(&path, body).unwrap();
    let id = first_id(&json_run(
        &db,
        "alice",
        &[
            "post",
            "create",
            "/work",
            "--title",
            "Plan",
            "--file",
            path.to_str().unwrap(),
        ],
    ));
    assert_eq!(
        json_run(&db, "bob", &["post", "show", &id])["items"][0]["body"],
        body
    );
    let plain = run(&db, "bob", &["post", "show", &id], None);
    assert!(plain.status.success());
    assert!(String::from_utf8(plain.stdout).unwrap().contains(body));
    let revised = "# Draft\nRevised 雪\n";
    let edit = run(
        &db,
        "alice",
        &[
            "post",
            "edit",
            &id,
            "--stdin",
            "--expected-revision",
            "1",
            "--json",
        ],
        Some(revised),
    );
    assert!(
        edit.status.success(),
        "{}",
        String::from_utf8_lossy(&edit.stderr)
    );
    let diff = json_run(&db, "bob", &["diff", &id]);
    assert!(diff.to_string().contains("Revised"));
    json_run(&db, "bob", &["state", "reset", &id]);
    json_run(&db, "bob", &["post", "show", &id, "--compact"]);
    let state = json_run(&db, "bob", &["state", "show", &id]);
    assert_eq!(state["items"][0]["seen_revision"], 2);
    assert_eq!(state["items"][0]["read_revision"], 0);
    assert_eq!(
        json_run(&db, "bob", &["post", "show", &id, "--full"])["items"][0]["body"],
        revised
    );
    assert_eq!(
        json_run(&db, "bob", &["state", "show", &id])["items"][0]["read_revision"],
        2
    );
    let comment = run(
        &db,
        "bob",
        &["comment", "create", &id, "--stdin", "--json"],
        Some("Comment\nwithout escaping"),
    );
    assert!(comment.status.success());
    let history = json_run(&db, "bob", &["history", &id]);
    assert_eq!(history["items"].as_array().unwrap().len(), 2);
}

#[test]
fn sql_render_and_config_overrides_are_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("board.db");
    let first = first_id(&json_run(
        &db,
        "alice",
        &["post", "create", "--title", "A", "--body", "Alpha"],
    ));
    json_run(
        &db,
        "alice",
        &["post", "create", "--title", "B", "--body", "Beta"],
    );
    json_run(&db, "alice", &["config", "set", "limit", "1"]);
    assert_eq!(
        json_run(&db, "bob", &["post", "list"])["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        json_run(&db, "bob", &["post", "list", "--limit", "2"])["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let selected = json_run(
        &db,
        "bob",
        &[
            "query",
            "SELECT id FROM posts ORDER BY id",
            "--render",
            "post",
            "--full",
            "--limit",
            "2",
        ],
    );
    assert_eq!(selected["items"].as_array().unwrap().len(), 2);
    assert_eq!(selected["items"][0]["body"], "Alpha");
    assert_eq!(
        json_run(&db, "bob", &["state", "show", &first])["items"][0]["read_revision"],
        1
    );
    let count = json_run(
        &db,
        "bob",
        &["query", "SELECT count(*) AS count FROM posts"],
    );
    assert_eq!(count["items"][0]["count"], 2);
    json_run(&db, "alice", &["config", "set", "format", "jsonl"]);
    let output = run(&db, "bob", &["post", "list"], None);
    assert!(output.status.success());
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(lines.last().unwrap()["type"], "meta");
    assert!(json_run(&db, "bob", &["post", "list"])["items"].is_array());
    json_run(&db, "alice", &["config", "set", "compact", "true"]);
    let full = run(
        &db,
        "bob",
        &["post", "show", &first, "--full", "--json"],
        None,
    );
    assert!(full.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&full.stdout).unwrap()["items"][0]["body"],
        "Alpha"
    );
}

#[test]
fn invalid_commands_are_actionable_and_logged_without_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("board.db");
    json_run(&db, "alice", &["init"]);
    let failure = run(&db, "alice", &["post", "show", "999"], None);
    assert!(!failure.status.success());
    assert!(!failure.stderr.is_empty());
    let failure = run(&db, "alice", &["post", "show"], None);
    assert_eq!(failure.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("Usage:"));
    let id = first_id(&json_run(
        &db,
        "alice",
        &[
            "post",
            "create",
            "--title",
            "Private payload",
            "--body",
            "PAYLOAD_SENTINEL",
        ],
    ));
    let failure = run(
        &db,
        "alice",
        &[
            "post",
            "edit",
            &id,
            "--body",
            "wrong",
            "--expected-revision",
            "9",
        ],
        None,
    );
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("revision conflict"));
    let logs = json_run(
        &db,
        "alice",
        &["log", "--author", "alice", "--limit", "100"],
    );
    assert!(
        logs["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["command"] == "cli.parse" && x["success"] == false)
    );
    assert!(!logs.to_string().contains("PAYLOAD_SENTINEL"));
    let failures = json_run(&db, "alice", &["log", "--failed"]);
    assert!(
        failures["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| x["success"] == json!(false))
    );
}

#[test]
fn budgeted_output_does_not_consume_full_read_state() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("board.db");
    let path = dir.path().join("large.txt");
    std::fs::write(&path, "Long body 雪\n".repeat(2000)).unwrap();
    let id = first_id(&json_run(
        &db,
        "alice",
        &[
            "post",
            "create",
            "--title",
            "Large",
            "--file",
            path.to_str().unwrap(),
        ],
    ));
    let output = run(
        &db,
        "bob",
        &["post", "show", &id, "--json", "--max-bytes", "2048"],
        None,
    );
    assert!(output.status.success());
    assert!(output.stdout.len() <= 2048);
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["more"], true);
    let state = json_run(&db, "bob", &["state", "show", &id]);
    assert!(
        state["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["read_revision"].as_i64().unwrap_or(0) == 0)
    );
    json_run(&db, "bob", &["post", "show", &id, "--max-bytes", "100000"]);
    assert_eq!(
        json_run(&db, "bob", &["state", "show", &id])["items"][0]["read_revision"],
        1
    );
}
