use agentboard::{
    db, events,
    model::{Output, Request},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn setup() -> Connection {
    let conn = db::open(":memory:").unwrap();
    for name in ["alice", "bob", "carol"] {
        db::ensure_agent(&conn, name).unwrap();
    }
    conn
}
fn command(conn: &mut Connection, actor: &str, name: &str, args: Value) -> Output {
    events::execute(
        conn,
        actor,
        &Request {
            command: name.into(),
            args,
        },
    )
    .unwrap()
}
fn post(conn: &Connection, author: &str, body: &str) -> i64 {
    conn.execute(
        "INSERT INTO objects(kind,forum_id,title,body,author) VALUES ('post',1,'A post',?1,?2)",
        params![body, author],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    db::save_revision(conn, id, author).unwrap();
    events::emit(conn, author, "post.created", id, &json!({})).unwrap();
    id
}
fn comment(conn: &Connection, author: &str, post: i64, reply: Option<i64>, body: &str) -> i64 {
    conn.execute("INSERT INTO objects(kind,forum_id,parent_id,reply_to,body,author) VALUES ('comment',1,?1,?2,?3,?4)",params![post,reply,body,author]).unwrap();
    let id = conn.last_insert_rowid();
    db::save_revision(conn, id, author).unwrap();
    events::emit(conn, author, "comment.created", id, &json!({})).unwrap();
    id
}

#[test]
fn overlapping_subscriptions_mentions_and_replies_deduplicate_and_retrieval_does_not_consume() {
    let mut conn = setup();
    let id = post(&conn, "alice", "hello");
    command(
        &mut conn,
        "alice",
        "subscribe",
        json!({"target_type":"forum","target":"/"}),
    );
    comment(&conn, "bob", id, None, "@alice please inspect");
    let inbox = command(&mut conn, "alice", "inbox", json!({}));
    assert_eq!(inbox.items.len(), 1);
    let reasons = inbox.items[0]["reasons"].as_array().unwrap();
    for expected in ["reply", "mention", "subscription:forum:1"] {
        assert!(reasons.contains(&json!(expected)));
    }
    assert!(reasons.contains(&json!(format!("subscription:post:{id}"))));
    assert_eq!(inbox.notification_ids.len(), 1);
    assert_eq!(
        command(&mut conn, "alice", "inbox", json!({})).items.len(),
        1
    );
    assert_eq!(
        command(&mut conn, "bob", "feed", json!({})).items.len(),
        0,
        "self activity should not notify"
    );
    conn.execute(
        "UPDATE notifications SET seen_at='ack' WHERE id=?1",
        [inbox.notification_ids[0]],
    )
    .unwrap();
    assert!(
        command(&mut conn, "alice", "feed", json!({}))
            .items
            .is_empty()
    );
    assert_eq!(
        command(&mut conn, "alice", "feed", json!({"all":true}))
            .items
            .len(),
        1
    );
}

#[test]
fn unsubscribe_survives_later_participation_and_explicit_subscribe_restores() {
    let mut conn = setup();
    let id = post(&conn, "alice", "");
    comment(&conn, "bob", id, None, "joining");
    command(
        &mut conn,
        "bob",
        "unsubscribe",
        json!({"target_type":"post","target":id}),
    );
    comment(&conn, "bob", id, None, "one last comment");
    comment(&conn, "carol", id, None, "new comment");
    assert!(
        command(&mut conn, "bob", "feed", json!({}))
            .items
            .is_empty()
    );
    command(
        &mut conn,
        "bob",
        "subscribe",
        json!({"target_type":"post","target":id,"inbox":true}),
    );
    comment(&conn, "carol", id, None, "another");
    assert_eq!(command(&mut conn, "bob", "inbox", json!({})).items.len(), 1);
}

#[test]
fn mention_can_precede_registration_and_edit_does_not_renotify_unchanged_mention() {
    let mut conn = setup();
    let id = post(
        &conn,
        "alice",
        "@future-agent welcome; alice@example.com is email",
    );
    assert_eq!(
        command(&mut conn, "future-agent", "inbox", json!({}))
            .items
            .len(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM agents WHERE name='example.com'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.execute(
        "UPDATE objects SET revision=revision+1,body=body||' edited' WHERE id=?1",
        [id],
    )
    .unwrap();
    db::save_revision(&conn, id, "alice").unwrap();
    events::emit(&conn, "alice", "post.edited", id, &json!({})).unwrap();
    assert_eq!(
        command(&mut conn, "future-agent", "inbox", json!({}))
            .items
            .len(),
        1
    );
}

#[test]
fn tag_agent_nested_forum_and_reply_to_comment_routing() {
    let mut conn = setup();
    command(
        &mut conn,
        "bob",
        "subscribe",
        json!({"target_type":"tag","target":"rust"}),
    );
    command(
        &mut conn,
        "carol",
        "subscribe",
        json!({"target_type":"agent","target":"alice"}),
    );
    command(
        &mut conn,
        "bob",
        "subscribe",
        json!({"target_type":"forum","target":"/"}),
    );
    conn.execute("INSERT INTO objects(kind,path,parent_id,title,author) VALUES ('forum','/child',1,'Child','alice')",[]).unwrap();
    let forum = conn.last_insert_rowid();
    let id = post(&conn, "alice", "");
    conn.execute(
        "UPDATE objects SET forum_id=?1 WHERE id=?2",
        params![forum, id],
    )
    .unwrap();
    conn.execute("INSERT INTO tags(object_id,tag) VALUES (?1,'rust')", [id])
        .unwrap();
    events::emit(&conn, "alice", "post.edited", id, &json!({})).unwrap();
    let feed = command(&mut conn, "bob", "feed", json!({}));
    assert!(
        feed.items[0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("subscription:tag:rust"))
    );
    assert!(
        feed.items[0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("subscription:forum:1"))
    );
    assert_eq!(
        command(&mut conn, "carol", "feed", json!({})).items.len(),
        2
    );
    let reply = comment(&conn, "bob", id, None, "a comment");
    comment(&conn, "carol", id, Some(reply), "response");
    let inbox = command(&mut conn, "bob", "inbox", json!({}));
    assert_eq!(inbox.items.len(), 1);
    assert!(
        inbox.items[0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("reply"))
    );
}

#[test]
fn limit_receipts_and_wait_leave_unreturned_notifications_pending() {
    let mut conn = setup();
    post(&conn, "alice", "@bob one");
    post(&conn, "alice", "@bob two");
    let feed = command(&mut conn, "bob", "feed", json!({"limit":1}));
    assert!(feed.more);
    assert_eq!(feed.notification_ids.len(), 1);
    let wake = command(&mut conn, "bob", "wait", json!({"timeout":0}));
    assert_eq!(wake.items[0]["ready"], true);
    assert_eq!(command(&mut conn, "bob", "feed", json!({})).items.len(), 2);
    assert!(
        command(&mut conn, "bob", "activity", json!({}))
            .notification_ids
            .is_empty()
    );
}

fn task(conn: &Connection, owner: Option<&str>) -> i64 {
    let id = post(conn, "alice", "");
    conn.execute(
        "INSERT INTO tasks(object_id,owner,status) VALUES (?1,?2,?3)",
        params![id, owner, if owner.is_some() { "claimed" } else { "open" }],
    )
    .unwrap();
    events::emit(conn, "alice", "task.assigned", id, &json!({"owner":owner})).unwrap();
    id
}

#[test]
fn dependencies_wait_for_every_completion_but_cancel_wakes_immediately_and_notifies_owner() {
    let mut conn = setup();
    let a = task(&conn, None);
    let b = task(&conn, None);
    let dependent = task(&conn, Some("bob"));
    for prerequisite in [a, b] {
        conn.execute(
            "INSERT INTO dependencies VALUES (?1,?2)",
            params![dependent, prerequisite],
        )
        .unwrap();
    }
    conn.execute("UPDATE tasks SET status='done' WHERE object_id=?1", [a])
        .unwrap();
    let waiting = command(
        &mut conn,
        "bob",
        "wait",
        json!({"dependencies":dependent,"timeout":0}),
    );
    assert_eq!(waiting.items[0]["ready"], false);
    assert_eq!(waiting.items[0]["remaining"], json!([b]));
    conn.execute(
        "UPDATE tasks SET status='cancelled' WHERE object_id=?1",
        [b],
    )
    .unwrap();
    events::emit(&conn, "alice", "task.cancelled", b, &json!({})).unwrap();
    let cancelled = command(
        &mut conn,
        "bob",
        "wait",
        json!({"dependencies":dependent,"timeout":0}),
    );
    assert_eq!(
        cancelled.items[0]["reasons"][0]["reason"],
        "dependency_cancelled"
    );
    let feed = command(&mut conn, "bob", "inbox", json!({}));
    assert!(
        feed.items[0]["reasons"]
            .as_array()
            .unwrap()
            .contains(&json!(format!("dependency:{dependent}")))
    );
    conn.execute("UPDATE tasks SET status='done' WHERE object_id=?1", [b])
        .unwrap();
    assert_eq!(
        command(
            &mut conn,
            "bob",
            "wait",
            json!({"dependencies":dependent,"timeout":0})
        )
        .items[0]["reasons"][0]["reason"],
        "dependencies_ready"
    );
}

#[test]
fn wait_observes_another_connection_without_consuming() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let mut conn = db::open(&path).unwrap();
    db::ensure_agent(&conn, "bob").unwrap();
    let writer = std::thread::spawn(move || {
        let conn = db::open(path).unwrap();
        db::ensure_agent(&conn, "alice").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(60));
        post(&conn, "alice", "@bob wake up");
    });
    let result = command(
        &mut conn,
        "bob",
        "wait",
        json!({"inbox":true,"timeout":2,"poll_ms":10}),
    );
    writer.join().unwrap();
    assert_eq!(result.items[0]["ready"], true);
    assert_eq!(command(&mut conn, "bob", "inbox", json!({})).items.len(), 1);
}

#[test]
fn event_and_notification_rollback_together() {
    let mut conn = setup();
    let tx = conn.transaction().unwrap();
    post(&tx, "alice", "@bob transaction");
    tx.rollback().unwrap();
    assert!(
        command(&mut conn, "bob", "inbox", json!({}))
            .items
            .is_empty()
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn wait_selectors_are_or_and_discovery_is_distinct_from_body_reading() {
    let mut conn = setup();
    let id = post(&conn, "alice", "body");
    let initial = command(&mut conn, "bob", "wait", json!({"post":id,"timeout":0}));
    assert_eq!(initial.items[0]["ready"], true);
    conn.execute(
        "INSERT INTO view_state(agent,object_id,seen_revision,read_revision) VALUES ('bob',?1,1,0)",
        [id],
    )
    .unwrap();
    let observed = command(&mut conn, "bob", "wait", json!({"post":id,"timeout":0}));
    assert_eq!(
        observed.items[0]["ready"], false,
        "unread body must not repeatedly wake discovery"
    );
    conn.execute("UPDATE objects SET revision=2 WHERE id=?1", [id])
        .unwrap();
    assert_eq!(
        command(&mut conn, "bob", "wait", json!({"post":id,"timeout":0})).items[0]["ready"],
        true
    );
    let pending = task(&conn, Some("alice"));
    let prerequisite = task(&conn, Some("alice"));
    conn.execute(
        "INSERT INTO dependencies VALUES (?1,?2)",
        params![pending, prerequisite],
    )
    .unwrap();
    post(&conn, "alice", "@bob needed elsewhere");
    assert_eq!(
        command(
            &mut conn,
            "bob",
            "wait",
            json!({"inbox":true,"dependencies":pending,"timeout":0})
        )
        .items[0]["reasons"][0]["reason"],
        "inbox"
    );
    assert_eq!(
        command(
            &mut conn,
            "bob",
            "wait",
            json!({"inbox":false,"subscriptions":false,"task_ready":false,"timeout":0})
        )
        .items[0]["ready"],
        true,
        "CLI false defaults must preserve default notification wait"
    );
}

#[test]
fn removed_tag_subscribers_receive_the_removal_event() {
    let mut conn = setup();
    command(
        &mut conn,
        "bob",
        "subscribe",
        json!({"target_type":"tag","target":"rust"}),
    );
    let id = post(&conn, "alice", "");
    conn.execute("INSERT INTO tags VALUES (?1,'rust')", [id])
        .unwrap();
    conn.execute("UPDATE objects SET revision=2 WHERE id=?1", [id])
        .unwrap();
    db::save_revision(&conn, id, "alice").unwrap();
    events::emit(&conn, "alice", "post.edited", id, &json!({})).unwrap();
    conn.execute("DELETE FROM tags WHERE object_id=?1", [id])
        .unwrap();
    conn.execute("UPDATE objects SET revision=3 WHERE id=?1", [id])
        .unwrap();
    db::save_revision(&conn, id, "alice").unwrap();
    events::emit(&conn, "alice", "post.edited", id, &json!({})).unwrap();
    assert_eq!(command(&mut conn, "bob", "feed", json!({})).items.len(), 2);
}

#[test]
fn forum_and_task_ready_waits_validate_current_state() {
    let mut conn = setup();
    let id = post(&conn, "alice", "");
    assert_eq!(
        command(&mut conn, "bob", "wait", json!({"forum":"/","timeout":0})).items[0]["ready"],
        true
    );
    conn.execute(
        "INSERT INTO view_state(agent,object_id,seen_revision) VALUES ('bob',?1,1)",
        [id],
    )
    .unwrap();
    assert_eq!(
        command(&mut conn, "bob", "wait", json!({"forum":"/","timeout":0})).items[0]["ready"],
        false
    );
    let task_id = task(&conn, None);
    assert_eq!(
        command(
            &mut conn,
            "bob",
            "wait",
            json!({"task_ready":true,"timeout":0})
        )
        .items[0]["reasons"][0]["task_id"],
        task_id
    );
    conn.execute("UPDATE objects SET archived=1 WHERE id=?1", [task_id])
        .unwrap();
    assert_eq!(
        command(
            &mut conn,
            "bob",
            "wait",
            json!({"task_ready":true,"timeout":0})
        )
        .items[0]["ready"],
        false
    );
    for args in [
        json!({"post":999,"timeout":0}),
        json!({"dependencies":id,"timeout":0}),
        json!({"forum":"/missing","timeout":0}),
        json!({"timeout":-1}),
    ] {
        assert!(
            events::execute(
                &mut conn,
                "bob",
                &Request {
                    command: "wait".into(),
                    args
                }
            )
            .is_err()
        );
    }
}

#[test]
fn terminating_waited_task_wakes_even_when_prerequisites_are_incomplete() {
    let mut conn = setup();
    let prerequisite = task(&conn, None);
    let dependent = task(&conn, Some("bob"));
    conn.execute(
        "INSERT INTO dependencies VALUES (?1,?2)",
        params![dependent, prerequisite],
    )
    .unwrap();
    for status in ["cancelled", "done"] {
        conn.execute(
            "UPDATE tasks SET status=?1 WHERE object_id=?2",
            params![status, dependent],
        )
        .unwrap();
        let result = command(
            &mut conn,
            "bob",
            "wait",
            json!({"dependencies":dependent,"timeout":0}),
        );
        assert_eq!(
            result.items[0]["reasons"][0]["reason"],
            format!("task_{status}")
        );
    }
}

#[test]
fn archived_parent_forum_hides_descendants_from_wait_conditions() {
    let mut conn = setup();
    let id = task(&conn, None);
    conn.execute("INSERT INTO objects(kind,path,parent_id,title,author,archived) VALUES ('forum','/archived',1,'Archived','alice',1)",[]).unwrap();
    let forum = conn.last_insert_rowid();
    conn.execute(
        "UPDATE objects SET forum_id=?1 WHERE id=?2",
        params![forum, id],
    )
    .unwrap();
    for args in [
        json!({"task_ready":true,"timeout":0}),
        json!({"forum":"/","timeout":0}),
        json!({"post":id,"timeout":0}),
    ] {
        assert_eq!(
            command(&mut conn, "bob", "wait", args).items[0]["ready"],
            false
        );
    }
}
