use agentboard::{db, model::Request, tasks};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::sync::{Arc, Barrier};

fn run(
    conn: &mut Connection,
    actor: &str,
    command: &str,
    args: Value,
) -> anyhow::Result<agentboard::model::Output> {
    tasks::execute(
        conn,
        actor,
        &Request {
            command: command.into(),
            args,
        },
    )
}

fn new_task(conn: &mut Connection, title: &str) -> i64 {
    run(
        conn,
        "lead",
        "task.create",
        json!({"forum":"/","title":title,"body":"Work description"}),
    )
    .unwrap()
    .items[0]["id"]
        .as_i64()
        .unwrap()
}

#[test]
fn simultaneous_claims_have_exactly_one_winner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("board.db");
    let mut conn = db::open(&path).unwrap();
    let id = new_task(&mut conn, "One owner");
    let barrier = Arc::new(Barrier::new(8));
    let threads = (0..8)
        .map(|n| {
            let barrier = barrier.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                let mut conn = db::open(path).unwrap();
                barrier.wait();
                run(
                    &mut conn,
                    &format!("worker-{n}"),
                    "task.claim",
                    json!({"id":id}),
                )
                .is_ok()
            })
        })
        .collect::<Vec<_>>();
    let winners = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .filter(|won| *won)
        .count();
    assert_eq!(winners, 1);
    assert_eq!(db::get_object(&conn, id).unwrap()["revision"], 2);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM events WHERE object_id=?1 AND kind='task.claimed'",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn dependency_batch_cycle_rolls_back_every_edge_and_event() {
    let mut conn = db::open(":memory:").unwrap();
    let a = new_task(&mut conn, "A");
    let b = new_task(&mut conn, "B");
    let c = new_task(&mut conn, "C");
    run(&mut conn, "lead", "task.depend", json!({"id":c,"ids":[b]})).unwrap();
    let before: i64 = conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    let result = run(
        &mut conn,
        "lead",
        "task.depend",
        // A is inserted before C reveals the cycle, exercising rollback of a partial batch.
        json!({"id":b,"ids":[a,c]}),
    );
    assert!(result.unwrap_err().to_string().contains("cycle"));
    let edges: i64 = conn
        .query_row("SELECT count(*) FROM dependencies", [], |r| r.get(0))
        .unwrap();
    assert_eq!(edges, 1);
    let after: i64 = conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    let object = db::get_object(&conn, b).unwrap();
    let snapshot: String = conn
        .query_row(
            "SELECT snapshot FROM revisions WHERE object_id=?1 AND revision=?2",
            rusqlite::params![b, object["revision"].as_i64()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&snapshot).unwrap()["task"],
        object["task"]
    );
}

#[test]
fn dual_creation_syntax_and_removal_preserve_graph_history() {
    let mut conn = db::open(":memory:").unwrap();
    let prerequisite = new_task(&mut conn, "Before");
    let blocked = new_task(&mut conn, "After");
    let middle = run(
        &mut conn,
        "lead",
        "task.create",
        json!({"forum":"/","title":"Middle","depends_on":[prerequisite],"blocks":[blocked]}),
    )
    .unwrap()
    .items[0]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        db::get_object(&conn, middle).unwrap()["task"]["depends_on"],
        json!([prerequisite])
    );
    assert_eq!(
        db::get_object(&conn, blocked).unwrap()["task"]["depends_on"],
        json!([middle])
    );
    assert_eq!(db::get_object(&conn, prerequisite).unwrap()["revision"], 2);
    run(
        &mut conn,
        "lead",
        "task.block",
        json!({"id":prerequisite,"ids":[middle],"remove":true}),
    )
    .unwrap();
    assert_eq!(
        db::get_object(&conn, middle).unwrap()["task"]["depends_on"],
        json!([])
    );
    assert_eq!(
        db::get_object(&conn, prerequisite).unwrap()["task"]["blocks"],
        json!([])
    );
    assert_eq!(db::get_object(&conn, prerequisite).unwrap()["revision"], 3);
}

#[test]
fn readiness_cancel_reopen_and_explicit_takeover() {
    let mut conn = db::open(":memory:").unwrap();
    let a = new_task(&mut conn, "Prerequisite");
    let b = new_task(&mut conn, "Dependent");
    run(&mut conn, "lead", "task.depend", json!({"id":b,"ids":[a]})).unwrap();
    assert!(run(&mut conn, "worker", "task.claim", json!({"id":b})).is_err());
    run(&mut conn, "lead", "task.cancel", json!({"id":a})).unwrap();
    let blocked = run(&mut conn, "worker", "task.show", json!({"id":b})).unwrap();
    assert_eq!(blocked.items[0]["cancelled_prerequisites"], json!([a]));
    assert!(
        run(&mut conn, "worker", "task.ready", json!({}))
            .unwrap()
            .items
            .is_empty()
    );
    run(&mut conn, "lead", "task.reopen", json!({"id":a})).unwrap();
    run(&mut conn, "lead", "task.done", json!({"id":a})).unwrap();
    run(&mut conn, "worker", "task.claim", json!({"id":b})).unwrap();
    assert!(run(&mut conn, "replacement", "task.release", json!({"id":b})).is_err());
    assert!(run(&mut conn, "replacement", "task.done", json!({"id":b})).is_err());
    run(&mut conn, "replacement", "task.takeover", json!({"id":b})).unwrap();
    run(&mut conn, "replacement", "task.done", json!({"id":b})).unwrap();
    assert_eq!(
        db::get_object(&conn, b).unwrap()["task"]["owner"],
        "replacement"
    );
}

#[test]
fn failed_creation_and_stale_revision_leave_no_partial_state() {
    let mut conn = db::open(":memory:").unwrap();
    let before: i64 = conn
        .query_row("SELECT count(*) FROM objects", [], |r| r.get(0))
        .unwrap();
    assert!(
        run(
            &mut conn,
            "lead",
            "task.create",
            json!({"forum":"/","title":"Invalid","depends_on":[99999]})
        )
        .is_err()
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM objects", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    let id = new_task(&mut conn, "CAS");
    run(
        &mut conn,
        "lead",
        "task.claim",
        json!({"id":id,"expected_revision":1}),
    )
    .unwrap();
    assert!(
        run(
            &mut conn,
            "lead",
            "task.done",
            json!({"id":id,"expected_revision":1})
        )
        .unwrap_err()
        .to_string()
        .contains("revision conflict")
    );
    assert_eq!(
        db::get_object(&conn, id).unwrap()["task"]["status"],
        "claimed"
    );
}

#[test]
fn list_receipts_do_not_consume_full_bodies_or_modify_state() {
    let mut conn = db::open(":memory:").unwrap();
    let id = new_task(&mut conn, "Discover");
    let result = run(&mut conn, "reader", "task.list", json!({})).unwrap();
    assert_eq!(result.items[0]["id"], id);
    assert!(result.items[0].get("body").is_none());
    assert!(!result.receipts[0].full);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM view_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let full = run(&mut conn, "reader", "task.show", json!({"id":id})).unwrap();
    assert_eq!(full.items[0]["body"], "Work description");
    assert!(full.receipts[0].full);
}

#[test]
fn attach_preserves_post_and_assignment_delivers_attention() {
    let mut conn = db::open(":memory:").unwrap();
    db::ensure_agent(&conn, "lead").unwrap();
    let post=agentboard::content::execute(&mut conn,"lead",&Request{command:"post.create".into(),args:json!({"forum":"/","title":"Design decision","body":"Already discussed","tags":["design"]})}).unwrap().items[0]["id"].as_i64().unwrap();
    run(
        &mut conn,
        "lead",
        "task.attach",
        json!({"id":post,"owner":"worker","expected_revision":1}),
    )
    .unwrap();
    let object = db::get_object(&conn, post).unwrap();
    assert_eq!(object["body"], "Already discussed");
    assert_eq!(object["tags"], json!(["design"]));
    assert_eq!(object["revision"], 2);
    assert_eq!(object["task"]["owner"], "worker");
    assert_eq!(conn.query_row("SELECT count(*) FROM notifications n JOIN events e ON e.id=n.event_id WHERE n.agent='worker' AND n.inbox=1 AND e.kind='task.attached'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert!(run(&mut conn, "lead", "task.attach", json!({"id":post})).is_err());
}

#[test]
fn cycle_during_creation_rolls_back_new_post_and_external_revisions() {
    let mut conn = db::open(":memory:").unwrap();
    let existing = new_task(&mut conn, "Existing");
    let before_events: i64 = conn
        .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert!(
        run(
            &mut conn,
            "lead",
            "task.create",
            json!({"title":"Cycle","depends_on":[existing],"blocks":[existing]})
        )
        .is_err()
    );
    assert_eq!(db::get_object(&conn, existing).unwrap()["revision"], 1);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM tasks", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before_events
    );
}

#[test]
fn task_creation_records_reference_backlink_activity() {
    let mut conn = db::open(":memory:").unwrap();
    let target = new_task(&mut conn, "Related discussion");
    let source = run(
        &mut conn,
        "worker",
        "task.create",
        json!({"title":"Follow-up","body":format!("Based on #{target}")}),
    )
    .unwrap()
    .items[0]["id"]
        .as_i64()
        .unwrap();
    let detail: String = conn
        .query_row(
            "SELECT detail FROM events WHERE kind='reference.added' AND object_id=?1",
            [target],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&detail).unwrap()["source_id"],
        source
    );
}

#[test]
fn archived_task_cannot_be_claimed_or_reported_ready_even_with_all() {
    let mut conn = db::open(":memory:").unwrap();
    let id = new_task(&mut conn, "Archived work");
    agentboard::content::execute(
        &mut conn,
        "lead",
        &Request {
            command: "post.archive".into(),
            args: json!({"id":id}),
        },
    )
    .unwrap();
    assert!(run(&mut conn, "worker", "task.claim", json!({"id":id})).is_err());
    assert!(
        run(&mut conn, "worker", "task.ready", json!({"all":true}))
            .unwrap()
            .items
            .is_empty()
    );
}
