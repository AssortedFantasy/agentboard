use agentboard::{db, model::Request, query};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn board() -> Connection {
    let conn = db::open(":memory:").unwrap();
    db::ensure_agent(&conn, "reader").unwrap();
    conn.execute(
        "INSERT INTO objects(kind,path,title,author) VALUES('forum','/build','Build','alice')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO objects(kind,path,title,author) VALUES('forum','/build/rust','Rust','alice')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO objects(kind,path,title,author) VALUES('forum','/builder','Other','alice')",
        [],
    )
    .unwrap();
    for (forum, title, body, author, archived) in [
        (
            2,
            "Compiler notes",
            "Rust compiler catches mistakes.\nPreserve **this** exactly.",
            "alice",
            0,
        ),
        (
            3,
            "Compiler notes",
            "Rust compiler catches mistakes.\nPreserve **this** exactly.",
            "bob",
            0,
        ),
        (
            4,
            "Compiler archive",
            "Old compiler architecture",
            "alice",
            1,
        ),
        (4, "Other work", "Database reliability", "alice", 0),
    ] {
        conn.execute("INSERT INTO objects(kind,forum_id,title,body,author,archived) VALUES('post',?1,?2,?3,?4,?5)", params![forum,title,body,author,archived]).unwrap();
    }
    conn.execute("INSERT INTO tags VALUES(5,'decision')", [])
        .unwrap();
    conn.execute("INSERT INTO tasks(object_id) VALUES(5)", [])
        .unwrap();
    conn
}

fn run(
    conn: &mut Connection,
    command: &str,
    args: Value,
) -> anyhow::Result<agentboard::model::Output> {
    query::execute(
        conn,
        "reader",
        &Request {
            command: command.into(),
            args,
        },
    )
}

#[test]
fn full_sql_joins_aggregation_ctes_and_actor_view() {
    let mut conn = board();
    let output = run(&mut conn, "query", json!({"sql":"WITH counts AS (SELECT author,count(*) AS n FROM posts GROUP BY author) SELECT author,n,(SELECT name FROM me) AS reader FROM counts ORDER BY n DESC,author"})).unwrap();
    assert_eq!(
        output.items,
        vec![
            json!({"author":"alice","n":3,"reader":"reader"}),
            json!({"author":"bob","n":1,"reader":"reader"})
        ]
    );
    assert!(output.receipts.is_empty());
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT '; -- /* not a statement */' AS text; -- trailing comment"}),
    )
    .unwrap();
    assert_eq!(output.items[0]["text"], "; -- /* not a statement */");
}

#[test]
fn sql_rejects_side_effects_and_restores_connection_after_errors() {
    let mut conn = board();
    for sql in [
        "DELETE FROM objects",
        "UPDATE objects SET title='bad' RETURNING id",
        "INSERT INTO objects(kind,author) VALUES('post','bad') RETURNING id",
        "DROP TABLE tags",
        "CREATE TEMP TABLE bad(a)",
        "ATTACH ':memory:' AS bad",
        "PRAGMA writable_schema=ON",
        "PRAGMA query_only=OFF",
        "PRAGMA user_version=999",
        "SELECT load_extension('bad')",
        "SELECT 1; SELECT 2",
        "SELECT 1; DELETE FROM objects",
        "BEGIN",
        "VACUUM",
        "SELECT writefile('bad','bad')",
    ] {
        assert!(
            run(&mut conn, "query", json!({"sql":sql})).is_err(),
            "accepted {sql}"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM posts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            4
        );
        // Retrieval must not leave the authorizer/progress hook blocking normal commands.
        conn.execute(
            "INSERT OR REPLACE INTO config(key,value) VALUES('after_query','true')",
            [],
        )
        .unwrap();
    }
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        db::SCHEMA_VERSION
    );
}

#[test]
fn query_rows_and_bytes_are_bounded_without_sql_limit() {
    let mut conn = board();
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT id,title FROM posts ORDER BY id", "limit":2, "offset":1}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 2);
    assert_eq!(output.items[0]["id"], 6);
    assert!(output.more);
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT id,title FROM posts ORDER BY id", "max_bytes":40}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 1);
    assert!(output.more);
    assert!(output.notices.iter().any(|n| n.contains("byte budget")));
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT id FROM posts ORDER BY id LIMIT 2", "limit":2}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 2);
    assert!(!output.more);
}

#[test]
fn object_mode_requires_id_projection_and_tracks_only_rendered_content() {
    let mut conn = board();
    for args in [
        json!({"sql":"SELECT * FROM posts", "render":"post"}),
        json!({"sql":"SELECT title FROM posts", "render":"post"}),
        json!({"sql":"SELECT '5' AS id", "render":"post"}),
        json!({"sql":"SELECT NULL AS id", "render":"post"}),
        json!({"sql":"SELECT 2 AS id", "render":"post"}),
    ] {
        assert!(run(&mut conn, "query", args).is_err());
    }
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT id FROM posts ORDER BY id", "render":"post","limit":1}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 1);
    assert!(output.items[0].get("body").is_none());
    assert_eq!(output.receipts.len(), 1);
    assert!(!output.receipts[0].full);
    assert_eq!(output.receipts[0].object_id, 5);
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT id FROM task_view", "render":"task","full":true}),
    )
    .unwrap();
    assert_eq!(
        output.items[0]["body"],
        "Rust compiler catches mistakes.\nPreserve **this** exactly."
    );
    assert!(output.receipts[0].full);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM view_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "retrieval must defer receipts to successful output"
    );
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT id FROM posts", "render":"post","full":true,"max_bytes":1}),
    )
    .unwrap();
    assert!(output.items.is_empty());
    assert!(output.receipts.is_empty());
    assert!(output.more);
}

#[test]
fn query_timeout_and_large_values_fail_without_poisoning_following_commands() {
    let mut conn = board();
    let result = run(
        &mut conn,
        "query",
        json!({"sql":"WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n", "query_ms":1}),
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("interrupt"));
    assert!(
        run(
            &mut conn,
            "query",
            json!({"sql":"SELECT randomblob(100000000)"})
        )
        .is_err()
    );
    assert_eq!(
        run(&mut conn, "query", json!({"sql":"SELECT 42 AS answer"}))
            .unwrap()
            .items[0]["answer"],
        42
    );
    conn.execute("UPDATE objects SET summary='after timeout' WHERE id=5", [])
        .unwrap();
}

#[test]
fn fts_search_filters_ranking_and_content_updates() {
    let mut conn = board();
    let output = run(&mut conn, "search", json!({"text":"compiler"})).unwrap();
    assert_eq!(
        output
            .items
            .iter()
            .map(|i| i["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![5, 6]
    );
    assert!(
        output.items[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("[Compiler]")
    );
    assert!(output.receipts.iter().all(|r| !r.full));
    let output = run(
        &mut conn,
        "search",
        json!({"text":"compiler", "forum":"/build","recursive":true,"author":"bob"}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 1);
    assert_eq!(output.items[0]["id"], 6);
    let output = run(
        &mut conn,
        "search",
        json!({"text":"compiler", "forum":"/build","tag":"decision","kind":"task","full":true}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 1);
    assert!(output.receipts[0].full);
    assert!(output.items[0].get("body").is_some());
    let output = run(&mut conn, "search", json!({"text":"compiler", "all":true})).unwrap();
    assert_eq!(output.items.len(), 3);
    conn.execute(
        "UPDATE objects SET title='Changed',body='Unique replacement' WHERE id=5",
        [],
    )
    .unwrap();
    assert_eq!(
        run(&mut conn, "search", json!({"text":"compiler"}))
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        run(&mut conn, "search", json!({"text":"replacement"}))
            .unwrap()
            .items[0]["id"],
        5
    );
    assert!(run(&mut conn, "search", json!({"text":"\"unterminated"})).is_err());
}

#[test]
fn search_pagination_marks_more_and_never_receipts_an_omitted_item() {
    let mut conn = board();
    let output = run(&mut conn, "search", json!({"text":"compiler", "limit":1})).unwrap();
    assert_eq!(output.items.len(), 1);
    assert_eq!(output.receipts.len(), 1);
    assert!(output.more);
    let page = run(
        &mut conn,
        "search",
        json!({"text":"compiler", "limit":1,"offset":1}),
    )
    .unwrap();
    assert_ne!(output.items[0]["id"], page.items[0]["id"]);
    assert!(!page.more);
}

#[test]
fn search_scopes_respect_forum_segments_and_archive_selection() {
    let mut conn = board();
    conn.execute("INSERT INTO objects(kind,forum_id,title,body,author) VALUES('post',4,'Compiler outside','compiler','alice')", []).unwrap();
    let output = run(
        &mut conn,
        "search",
        json!({"text":"compiler", "forum":"/build","recursive":true,"all":true}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 2);
    let output = run(
        &mut conn,
        "search",
        json!({"text":"compiler", "archived":true}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 1);
    assert_eq!(output.items[0]["id"], 7);
    let output = run(
        &mut conn,
        "search",
        json!({"text":"Rust", "forum":"/build","recursive":true,"kind":"forum"}),
    )
    .unwrap();
    assert_eq!(output.items.len(), 1);
    assert_eq!(output.items[0]["path"], "/build/rust");
}

#[test]
fn table_output_retains_nulls_blobs_and_requires_unique_aliases() {
    let mut conn = board();
    let output = run(
        &mut conn,
        "query",
        json!({"sql":"SELECT NULL AS missing, x'0041ff' AS data, 1.5 AS real"}),
    )
    .unwrap();
    assert_eq!(
        output.items[0],
        json!({"missing":null,"data":{"blob_hex":"0041ff","bytes":3},"real":1.5})
    );
    assert!(
        run(
            &mut conn,
            "query",
            json!({"sql":"SELECT 1 AS same, 2 AS same"})
        )
        .is_err()
    );
    assert!(run(&mut conn, "query", json!({"sql":"SELECT ? AS unbound"})).is_err());
    let output = run(&mut conn, "query", json!({"sql":"SELECT name FROM me"})).unwrap();
    assert_eq!(output.items[0]["name"], "reader");
}

#[test]
fn search_inherits_archives_from_forum_ancestors_and_containing_post() {
    let mut conn = board();
    conn.execute("UPDATE objects SET parent_id=2 WHERE id=3", [])
        .unwrap();
    conn.execute("UPDATE objects SET archived=1 WHERE id=2", [])
        .unwrap();
    assert!(
        run(&mut conn, "search", json!({"text":"compiler"}))
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        run(
            &mut conn,
            "search",
            json!({"text":"compiler","archived":true})
        )
        .unwrap()
        .items
        .len(),
        3
    );
    assert_eq!(
        run(&mut conn, "search", json!({"text":"compiler","all":true}))
            .unwrap()
            .items
            .len(),
        3
    );
    conn.execute("UPDATE objects SET archived=0 WHERE id=2", [])
        .unwrap();
    conn.execute("INSERT INTO objects(kind,parent_id,forum_id,body,author) VALUES('comment',5,2,'Unique reply text','bob')", []).unwrap();
    assert_eq!(
        run(&mut conn, "search", json!({"text":"Unique"}))
            .unwrap()
            .items
            .len(),
        1
    );
    conn.execute("UPDATE objects SET archived=1 WHERE id=5", [])
        .unwrap();
    assert!(
        run(&mut conn, "search", json!({"text":"Unique"}))
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        run(
            &mut conn,
            "search",
            json!({"text":"Unique","archived":true})
        )
        .unwrap()
        .items
        .len(),
        1
    );
}

#[test]
fn app_transactions_and_query_guards_survive_success_failure_and_interruption() {
    use agentboard::{app, render};
    let mut conn = board();
    for sql in [
        "SELECT id,revision,body,kind,title FROM posts ORDER BY id",
        "SELECT 1; DELETE FROM objects",
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",
    ] {
        let request = Request {
            command: "query".into(),
            args: json!({"sql":sql,"query_ms":1}),
        };
        let result = app::execute(&mut conn, "reader", &request);
        if sql.starts_with("SELECT id") {
            let rendered = render::render(result.as_ref().unwrap(), "json", false, 65_536).unwrap();
            assert!(
                rendered.receipts.is_empty(),
                "arbitrary table columns must not manufacture read receipts"
            );
            app::acknowledge(&mut conn, "reader", &rendered).unwrap();
        } else {
            assert!(result.is_err());
        }
        assert!(
            conn.is_autocommit(),
            "app must commit or roll back its read transaction"
        );
        app::log_command(&conn, "reader", &request, &result, 1).unwrap();
        app::execute(
            &mut conn,
            "reader",
            &Request {
                command: "config.set".into(),
                args: json!({"key":"limit","value":25}),
            },
        )
        .unwrap();
    }
    assert_eq!(
        conn.query_row("SELECT count(*) FROM command_log", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM view_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let output = app::execute(
        &mut conn,
        "reader",
        &Request {
            command: "search".into(),
            args: json!({"text":"compiler","full":true}),
        },
    )
    .unwrap();
    let rendered = render::render(&output, "json", false, 65_536).unwrap();
    app::acknowledge(&mut conn, "reader", &rendered).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM view_state WHERE read_revision=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert!(conn.is_autocommit());
}
