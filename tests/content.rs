use agentboard::{
    content, db,
    model::{Output, Request},
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn board() -> Connection {
    let conn = db::open(":memory:").unwrap();
    for actor in ["alice", "bob"] {
        db::ensure_agent(&conn, actor).unwrap();
    }
    conn
}
fn run(conn: &mut Connection, actor: &str, command: &str, args: Value) -> Output {
    content::execute(
        conn,
        actor,
        &Request {
            command: command.into(),
            args,
        },
    )
    .unwrap()
}
fn post(conn: &mut Connection, title: &str, body: &str) -> i64 {
    run(
        conn,
        "alice",
        "post.create",
        json!({"title":title,"body":body}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap()
}
fn fail(conn: &mut Connection, command: &str, args: Value) -> String {
    content::execute(
        conn,
        "alice",
        &Request {
            command: command.into(),
            args,
        },
    )
    .unwrap_err()
    .to_string()
}
fn read(conn: &Connection, actor: &str, id: i64, seen: i64, read: i64) {
    conn.execute("INSERT OR REPLACE INTO view_state(agent,object_id,seen_revision,read_revision) VALUES(?1,?2,?3,?4)",params![actor,id,seen,read]).unwrap();
}

#[test]
fn revisions_are_immutable_and_cas_failure_rolls_back_everything() {
    let mut conn = board();
    let id = post(&mut conn, "Plan", "one");
    run(
        &mut conn,
        "alice",
        "post.edit",
        json!({"id":id,"body":"two","tags":["decision"],"metadata":{"confidence":0.9},"expected_revision":1}),
    );
    let error = fail(
        &mut conn,
        "post.edit",
        json!({"id":id,"body":"bad","tags":["wrong"],"expected_revision":1}),
    );
    assert!(error.contains("revision conflict"));
    assert_eq!(db::get_object(&conn, id).unwrap()["body"], "two");
    let history = run(&mut conn, "bob", "history", json!({"id":id,"full":true}));
    assert_eq!(history.items.len(), 2);
    assert_eq!(history.items[1]["snapshot"]["body"], "one");
    assert_eq!(history.items[0]["snapshot"]["tags"], json!(["decision"]));
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM events WHERE object_id=?1",
            [id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn net_diff_uses_last_read_not_last_seen_and_avoids_replaying_intermediates() {
    let mut conn = board();
    let id = post(&mut conn, "Plan", "start");
    read(&conn, "bob", id, 1, 1);
    run(
        &mut conn,
        "alice",
        "post.edit",
        json!({"id":id,"body":"middle"}),
    );
    run(
        &mut conn,
        "alice",
        "post.edit",
        json!({"id":id,"body":"finish"}),
    );
    read(&conn, "bob", id, 3, 1);
    let diff = run(&mut conn, "bob", "diff", json!({"id":id}));
    assert_eq!(
        diff.items[0]["changes"]["fields"]["body"],
        json!({"before":"start","after":"finish"})
    );
    assert!(diff.receipts[0].full);
    assert_eq!(diff.receipts[0].revision, 3);
    let unrelated = run(&mut conn, "bob", "diff", json!({"id":id,"from":2}));
    assert!(unrelated.receipts.is_empty());
    let historical = run(&mut conn, "bob", "diff", json!({"id":id,"to":2}));
    assert!(historical.receipts.is_empty());
}

#[test]
fn lists_prepare_seen_receipts_and_do_not_mutate_agent_state() {
    let mut conn = board();
    let id = post(&mut conn, "Plan", "large secret body");
    let updates = run(&mut conn, "bob", "updates", json!({"kind":"post"}));
    assert_eq!(updates.items.len(), 1);
    assert_eq!(updates.items[0]["body_omitted"], true);
    assert!(updates.items[0].get("body").is_none());
    assert!(!updates.items[0].to_string().contains("large secret body"));
    assert!(!updates.receipts[0].full);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM view_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    read(&conn, "bob", id, 1, 0);
    assert!(
        run(&mut conn, "bob", "updates", json!({"kind":"post"}))
            .items
            .is_empty()
    );
    assert!(run(&mut conn, "bob", "post.show", json!({"id":id})).receipts[0].full);
}

#[test]
fn references_create_backlinks_and_update_without_polluting_authored_text() {
    let mut conn = board();
    let target = post(&mut conn, "Referenced", "original");
    let source = post(&mut conn, "Source", &format!("See #{target}."));
    let backlinks = run(&mut conn, "bob", "backlinks", json!({"id":target}));
    assert_eq!(backlinks.items[0]["id"], source);
    assert_eq!(
        db::get_object(&conn, source).unwrap()["body"],
        format!("See #{target}.")
    );
    let thread = run(&mut conn, "bob", "thread", json!({"id":target}));
    assert!(
        thread
            .items
            .iter()
            .any(|v| v["event_kind"] == "reference.added")
    );
    run(
        &mut conn,
        "alice",
        "post.edit",
        json!({"id":source,"body":"Reference removed"}),
    );
    assert!(
        run(&mut conn, "bob", "backlinks", json!({"id":target}))
            .items
            .is_empty()
    );
    assert_eq!(
        content::references("#12, (#13) abc#14 ##15 #16x #17_ #18 #999999999999999999999999"),
        [12, 13, 18].into()
    );
}

#[test]
fn nested_forums_and_filtered_pagination_preserve_boundaries() {
    let mut conn = board();
    run(&mut conn, "alice", "forum.create", json!({"path":"/code"}));
    run(
        &mut conn,
        "alice",
        "forum.create",
        json!({"path":"/code/parser"}),
    );
    assert!(
        fail(&mut conn, "forum.create", json!({"path":"/missing/child"}))
            .contains("does not exist")
    );
    for path in ["/code", "/code/parser"] {
        run(
            &mut conn,
            "alice",
            "post.create",
            json!({"forum":path,"title":path,"tags":["work"]}),
        );
    }
    let shallow = run(&mut conn, "bob", "post.list", json!({"forum":"/code"}));
    assert_eq!(shallow.items.len(), 1);
    let page = run(
        &mut conn,
        "bob",
        "post.list",
        json!({"forum":"/code","recursive":true,"tag":"work","limit":1}),
    );
    assert!(page.more);
    assert_eq!(page.items.len(), 1);
    let page2 = run(
        &mut conn,
        "bob",
        "post.list",
        json!({"forum":"/code","recursive":true,"tag":"work","limit":1,"offset":1}),
    );
    assert!(!page2.more);
    assert_ne!(page.items[0]["id"], page2.items[0]["id"]);
}

#[test]
fn comments_have_chronological_and_focused_thread_views() {
    let mut conn = board();
    let id = post(&mut conn, "Topic", "Discuss");
    let other = post(&mut conn, "Other", "Elsewhere");
    let c1 = run(
        &mut conn,
        "alice",
        "comment.create",
        json!({"post":id,"body":"first"}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap();
    let c2 = run(
        &mut conn,
        "bob",
        "comment.create",
        json!({"post":id,"reply_to":c1,"body":"reply"}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap();
    run(
        &mut conn,
        "alice",
        "comment.create",
        json!({"post":id,"body":"separate"}),
    );
    assert!(
        fail(
            &mut conn,
            "comment.create",
            json!({"post":other,"reply_to":c1,"body":"wrong post"})
        )
        .contains("another post")
    );
    let timeline = run(&mut conn, "alice", "thread", json!({"id":id}));
    let comments: Vec<_> = timeline
        .items
        .iter()
        .filter(|v| v["kind"] == "comment")
        .collect();
    assert_eq!(comments.len(), 3);
    assert_eq!(comments[0]["id"], c1);
    assert_eq!(comments[1]["id"], c2);
    let focused = run(&mut conn, "alice", "thread", json!({"id":c1}));
    assert_eq!(focused.items.len(), 2);
    assert_eq!(focused.items[1]["depth"], 1);
}

#[test]
fn archive_retains_history_and_references_and_can_be_restored() {
    let mut conn = board();
    let id = post(&mut conn, "Archive me", "body");
    run(&mut conn, "alice", "post.archive", json!({"id":id}));
    assert!(
        run(&mut conn, "bob", "post.list", json!({}))
            .items
            .is_empty()
    );
    assert_eq!(
        run(&mut conn, "bob", "post.list", json!({"archived":true}))
            .items
            .len(),
        1
    );
    assert_eq!(
        run(&mut conn, "bob", "post.show", json!({"id":id})).items[0]["body"],
        "body"
    );
    assert!(
        fail(
            &mut conn,
            "comment.create",
            json!({"post":id,"body":"late"})
        )
        .contains("archived")
    );
    run(&mut conn, "alice", "post.unarchive", json!({"id":id}));
    assert_eq!(run(&mut conn, "bob", "post.list", json!({})).items.len(), 1);
    assert!(fail(&mut conn, "forum.archive", json!({"path":"/"})).contains("root"));
}

#[test]
fn metadata_tags_and_summary_changes_are_versioned() {
    let mut conn = board();
    let id = post(&mut conn, "Document", "body");
    run(
        &mut conn,
        "alice",
        "tag.add",
        json!({"id":id,"tags":["design","design","rust"]}),
    );
    run(
        &mut conn,
        "alice",
        "tag.remove",
        json!({"id":id,"tags":["rust"]}),
    );
    run(
        &mut conn,
        "alice",
        "metadata.set",
        json!({"id":id,"metadata":{"nested":{"ok":true}}}),
    );
    run(
        &mut conn,
        "alice",
        "summary.set",
        json!({"id":id,"summary":"Short"}),
    );
    let item = db::get_object(&conn, id).unwrap();
    assert_eq!(item["revision"], 5);
    assert_eq!(item["tags"], json!(["design"]));
    assert_eq!(item["summary"], "Short");
    assert!(
        fail(&mut conn, "metadata.set", json!({"id":id,"metadata":[1,2]})).contains("JSON object")
    );
    assert_eq!(db::get_object(&conn, id).unwrap()["revision"], 5);
}

#[test]
fn inherited_archive_hides_descendants_without_rewriting_them() {
    let mut conn = board();
    let forum = run(&mut conn, "alice", "forum.create", json!({"path":"/work"})).items[0]["id"]
        .as_i64()
        .unwrap();
    run(
        &mut conn,
        "alice",
        "forum.create",
        json!({"path":"/work/sub"}),
    );
    let id = run(
        &mut conn,
        "alice",
        "post.create",
        json!({"forum":"/work/sub","title":"Nested"}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap();
    let comment = run(
        &mut conn,
        "bob",
        "comment.create",
        json!({"post":id,"body":"reply"}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap();
    run(&mut conn, "alice", "forum.archive", json!({"id":forum}));
    assert!(content::effectively_archived(&conn, comment).unwrap());
    assert_eq!(db::get_object(&conn, id).unwrap()["revision"], 1);
    assert!(
        run(&mut conn, "bob", "post.list", json!({}))
            .items
            .is_empty()
    );
    assert!(
        run(&mut conn, "bob", "comment.list", json!({"post":id}))
            .items
            .is_empty()
    );
    assert_eq!(
        run(&mut conn, "bob", "post.list", json!({"archived":true}))
            .items
            .len(),
        1
    );
    assert!(
        fail(
            &mut conn,
            "post.create",
            json!({"forum":"/work/sub","title":"Blocked"})
        )
        .contains("archived")
    );
    assert!(
        fail(
            &mut conn,
            "comment.create",
            json!({"post":id,"body":"Blocked"})
        )
        .contains("archived")
    );
    run(&mut conn, "alice", "forum.unarchive", json!({"id":forum}));
    assert_eq!(run(&mut conn, "bob", "post.list", json!({})).items.len(), 1);
    assert_eq!(
        run(
            &mut conn,
            "bob",
            "forum.list",
            json!({"forum":"/work","recursive":true})
        )
        .items
        .len(),
        1
    );
}

#[test]
fn full_updates_render_a_net_patch_and_keep_automatic_read_receipt() {
    let mut conn = board();
    let id = post(&mut conn, "Progress", "first\n");
    read(&conn, "bob", id, 1, 1);
    run(
        &mut conn,
        "alice",
        "post.edit",
        json!({"id":id,"body":"intermediate\n"}),
    );
    run(
        &mut conn,
        "alice",
        "post.edit",
        json!({"id":id,"body":"final\n"}),
    );
    let result = run(
        &mut conn,
        "bob",
        "updates",
        json!({"kind":"post","full":true}),
    );
    assert_eq!(result.items.len(), 1);
    assert!(result.items[0].get("body").is_none());
    let patch = result.items[0]["patch"].as_str().unwrap();
    assert!(patch.contains("-first"));
    assert!(patch.contains("+final"));
    assert!(!patch.contains("intermediate"));
    assert!(result.receipts[0].full);
}

#[test]
fn tree_keeps_visible_replies_when_their_parent_comment_is_archived() {
    let mut conn = board();
    let id = post(&mut conn, "Discuss", "body");
    let parent = run(
        &mut conn,
        "alice",
        "comment.create",
        json!({"post":id,"body":"parent"}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap();
    let child = run(
        &mut conn,
        "bob",
        "comment.create",
        json!({"post":id,"reply_to":parent,"body":"child"}),
    )
    .items[0]["id"]
        .as_i64()
        .unwrap();
    run(&mut conn, "alice", "comment.archive", json!({"id":parent}));
    let tree = run(&mut conn, "bob", "thread", json!({"id":id,"tree":true}));
    assert!(tree.items.iter().any(|v| v["id"] == child));
    assert!(!tree.items.iter().any(|v| v["id"] == parent));
}
