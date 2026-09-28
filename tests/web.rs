use agentboard::{db, web};
use rusqlite::{Connection, OpenFlags, params};

fn fixture(conn: &Connection) {
    db::ensure_agent(conn, "alice").unwrap();
    conn.execute(
        "UPDATE agents SET profile=?1 WHERE name='alice'",
        [r#"{"about":"<script>profile</script>"}"#],
    )
    .unwrap();
    conn.execute("INSERT INTO objects(id,kind,parent_id,path,title,author) VALUES(2,'forum',1,'/engineering','Engineering','alice')", []).unwrap();
    conn.execute("INSERT INTO objects(id,kind,forum_id,title,body,summary,metadata,author) VALUES(3,'post',2,?1,?2,'Useful summary',?3,'alice')", params!["<script>alert(1)</script>", "First line\n<script>body</script> & \"quoted\"", r#"{"priority":"high"}"#]).unwrap();
    conn.execute("INSERT INTO objects(id,kind,forum_id,title,body,author) VALUES(4,'post',2,'Prerequisite','Prepare the base','alice')", []).unwrap();
    conn.execute("INSERT INTO objects(id,kind,forum_id,parent_id,body,author,created_at) VALUES(5,'comment',2,3,'First reply','alice','2026-01-01T00:00:00Z')", []).unwrap();
    conn.execute("INSERT INTO objects(id,kind,forum_id,parent_id,reply_to,body,author,created_at) VALUES(6,'comment',2,3,5,'Second reply','alice','2026-01-01T00:00:02Z')", []).unwrap();
    conn.execute("INSERT INTO objects(id,kind,forum_id,title,author,archived) VALUES(7,'post',2,'Archived record','alice',1)", []).unwrap();
    conn.execute(
        "INSERT INTO tasks(object_id,status,owner) VALUES(3,'claimed','alice'),(4,'open',NULL)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO dependencies VALUES(3,4)", [])
        .unwrap();
    conn.execute("INSERT INTO tags VALUES(3,'a&b'),(4,'base')", [])
        .unwrap();
    conn.execute("INSERT INTO links VALUES(3,4),(4,3)", [])
        .unwrap();
    conn.execute("INSERT INTO events(actor,kind,object_id,post_id,detail,created_at) VALUES('alice','task.claimed',3,3,'{}','2026-01-01T00:00:01Z')", []).unwrap();
    db::save_revision(conn, 3, "alice").unwrap();
}

#[test]
fn pages_show_connected_content_and_escape_authored_html() {
    let conn = db::open(":memory:").unwrap();
    fixture(&conn);
    let (status, html) = web::render_route(&conn, "/objects/3").unwrap();
    assert_eq!(status, 200);
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(
        html.contains("First line\n&lt;script&gt;body&lt;/script&gt; &amp; &quot;quoted&quot;")
    );
    assert!(!html.contains("<script>"));
    for expected in [
        "Useful summary",
        "claimed",
        "Depends on",
        "Mentions",
        "Mentioned in",
        "Revision history",
        "Reply to",
        "System: task.claimed",
    ] {
        assert!(html.contains(expected), "missing {expected}");
    }
    assert!(html.find("First reply").unwrap() < html.find("System: task.claimed").unwrap());
    assert!(html.find("System: task.claimed").unwrap() < html.find("Second reply").unwrap());
    for url in [
        "/",
        "/forums",
        "/tasks",
        "/tags",
        "/tags/a%26b",
        "/agents",
        "/agents/alice",
        "/activity",
        "/archive",
        "/objects/2",
        "/history/3",
    ] {
        assert_eq!(web::render_route(&conn, url).unwrap().0, 200, "{url}");
    }
    assert!(
        web::render_route(&conn, "/objects/2")
            .unwrap()
            .1
            .contains("/engineering")
    );
    assert!(
        web::render_route(&conn, "/archive")
            .unwrap()
            .1
            .contains("Archived record")
    );
    assert!(
        !web::render_route(&conn, "/")
            .unwrap()
            .1
            .contains("Archived record")
    );
    assert!(
        web::render_route(&conn, "/agents/alice")
            .unwrap()
            .1
            .contains("&lt;script&gt;profile&lt;/script&gt;")
    );
    assert!(
        web::render_route(&conn, "/history/3")
            .unwrap()
            .1
            .contains("Revision 1")
    );
}

#[test]
fn browsing_works_with_read_only_database_and_never_advances_attention() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("board.db");
    let conn = db::open(&path).unwrap();
    fixture(&conn);
    conn.execute(
        "INSERT INTO view_state(agent,object_id,seen_revision,read_revision) VALUES('alice',3,0,0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notifications(agent,event_id,inbox) VALUES('alice',1,1)",
        [],
    )
    .unwrap();
    let read_only = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    for url in [
        "/",
        "/objects/3",
        "/history/3",
        "/agents/alice",
        "/activity",
    ] {
        assert_eq!(web::render_route(&read_only, url).unwrap().0, 200);
    }
    assert_eq!(read_only.total_changes(), 0);
    assert_eq!(
        conn.query_row(
            "SELECT read_revision+seen_revision FROM view_state",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT seen_at FROM notifications", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM command_log", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn pagination_is_bounded_and_routes_reject_invalid_input() {
    let conn = db::open(":memory:").unwrap();
    for index in 0..55 {
        conn.execute(
            "INSERT INTO objects(kind,forum_id,title,author) VALUES('post',1,?1,'system')",
            [format!("Unique title {index:02}")],
        )
        .unwrap();
    }
    let (_, first) = web::render_route(&conn, "/").unwrap();
    assert_eq!(first.matches("<article ").count(), 50);
    assert!(first.contains("Next (more results)"));
    let (_, second) = web::render_route(&conn, "/?offset=50").unwrap();
    assert_eq!(second.matches("<article ").count(), 5);
    assert!(second.contains("Previous"));
    assert!(!second.contains("Next (more results)"));
    for url in [
        "/missing",
        "/objects/999",
        "/objects/1%20OR%201=1",
        "/history/999",
        "/tags/%FF",
        "/agents/missing",
    ] {
        assert_eq!(web::render_route(&conn, url).unwrap().0, 404, "{url}");
    }
    for url in [
        "/?offset=-1",
        "/?offset=no",
        "/?offset=999999999999999999999",
    ] {
        assert_eq!(web::render_route(&conn, url).unwrap().0, 400, "{url}");
    }
}

#[test]
fn forum_archive_hides_descendants_from_active_views_but_retains_browsing() {
    let conn = db::open(":memory:").unwrap();
    fixture(&conn);
    conn.execute("UPDATE objects SET archived=1 WHERE id=2", [])
        .unwrap();
    for route in ["/", "/tasks", "/tags", "/tags/base"] {
        let (_, html) = web::render_route(&conn, route).unwrap();
        assert!(!html.contains("Prerequisite"), "{route}");
        assert!(!html.contains("href=\"/tags/base\""), "{route}");
    }
    let (_, archived) = web::render_route(&conn, "/archive").unwrap();
    assert!(archived.contains("Prerequisite"));
    assert!(archived.contains("archived through parent"));
    let (_, forum) = web::render_route(&conn, "/objects/2").unwrap();
    assert!(forum.contains("Prerequisite"));
    assert!(
        web::render_route(&conn, "/objects/3")
            .unwrap()
            .1
            .contains("First reply")
    );
}
