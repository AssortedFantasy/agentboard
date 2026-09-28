//! Exercise the real binary and TCP server, including the non-mutating serve path.
use agentboard::db;
use rusqlite::Connection;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ServerProcess {
    child: Child,
    address: SocketAddr,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(path: &Path) -> ServerProcess {
    // Reserve an OS-selected port briefly, then pass it to the real CLI.
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    spawn_at(path, address)
}

fn spawn_at(path: &Path, address: SocketAddr) -> ServerProcess {
    let child = Command::new(env!("CARGO_BIN_EXE_agentboard"))
        .arg(path)
        .args(["serve", "--bind", &address.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    ServerProcess { child, address }
}

fn ready(server: &mut ServerProcess) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            let mut error = String::new();
            server
                .child
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut error)
                .unwrap();
            panic!("server exited before startup: {status}; {error}");
        }
        if TcpStream::connect_timeout(&server.address, Duration::from_millis(100)).is_ok() {
            return;
        }
        assert!(Instant::now() < deadline, "server startup timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

fn request(server: &ServerProcess, method: &str, route: &str) -> String {
    let mut socket = TcpStream::connect_timeout(&server.address, Duration::from_secs(2)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        socket,
        "{method} {route} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        server.address
    )
    .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    response
}

fn fingerprint(conn: &Connection) -> String {
    let mut result = String::new();
    for table in ["agents", "view_state", "notifications"] {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let columns = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for col in 0..columns {
                result.push_str(&format!("{:?}|", row.get_ref(col).unwrap()));
            }
            result.push('\n');
        }
    }
    result
}

#[test]
fn real_http_server_serves_safe_pages_without_mutating_board() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let conn = db::open(&path).unwrap();
    db::ensure_agent(&conn, "alice").unwrap();
    conn.execute("INSERT INTO objects(id,kind,forum_id,title,body,author) VALUES(2,'post',1,'HTTP example','<script>unsafe</script>','alice')", []).unwrap();
    conn.execute(
        "INSERT INTO events(actor,kind,object_id,post_id) VALUES('alice','post.created',2,2)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO view_state(agent,object_id) VALUES('alice',2)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notifications(agent,event_id,inbox) VALUES('alice',1,1)",
        [],
    )
    .unwrap();
    db::save_revision(&conn, 2, "alice").unwrap();
    let before = fingerprint(&conn);
    let mut server = spawn(&path);
    ready(&mut server);
    let get = request(&server, "GET", "/objects/2");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM command_log WHERE command='serve' AND success=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(get.starts_with("HTTP/1.1 200"), "{get}");
    assert!(get.contains("HTTP example"));
    assert!(get.contains("&lt;script&gt;unsafe&lt;/script&gt;"));
    assert!(get.contains("href=\"/history/2\""));
    assert!(
        get.to_lowercase()
            .contains("content-type: text/html; charset=utf-8")
    );
    assert!(
        get.to_lowercase()
            .contains("content-security-policy: default-src 'none'")
    );
    assert!(get.to_lowercase().contains("cache-control: no-store"));
    for route in [
        "/",
        "/forums",
        "/tasks",
        "/tags",
        "/agents",
        "/activity",
        "/archive",
        "/history/2",
    ] {
        assert!(
            request(&server, "GET", route).starts_with("HTTP/1.1 200"),
            "{route}"
        );
    }
    let head = request(&server, "HEAD", "/objects/2");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert_eq!(head.split_once("\r\n\r\n").unwrap().1, "");
    let post = request(&server, "POST", "/objects/2");
    assert!(post.starts_with("HTTP/1.1 405"));
    assert!(post.to_lowercase().contains("allow: get, head"));
    assert!(request(&server, "GET", "/objects/999").starts_with("HTTP/1.1 404"));
    assert!(request(&server, "GET", "/?offset=-1").starts_with("HTTP/1.1 400"));
    assert_eq!(fingerprint(&conn), before);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM command_log", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(server);
}

fn assert_startup_rejected(path: &Path, expected: &str) {
    assert_rejected(spawn(path), expected);
}

fn assert_rejected(mut server: ServerProcess, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(!status.success());
            let mut stderr = String::new();
            server
                .child
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            assert!(stderr.contains(expected), "expected {expected}: {stderr}");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "invalid board unexpectedly served"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn occupied_port_records_failed_launch_without_mutating_attention() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let conn = db::open(&path).unwrap();
    let before = fingerprint(&conn);
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = spawn_at(&path, occupied.local_addr().unwrap());
    assert_rejected(server, "bind");
    assert_eq!(fingerprint(&conn), before);
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM command_log WHERE command='serve' AND success=0 AND error LIKE '%bind%'", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
}

#[test]
fn serve_rejects_missing_and_unsupported_boards_without_creating_or_migrating() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.db");
    assert_startup_rejected(&missing, "read-only");
    assert!(!missing.exists());

    let future = dir.path().join("future.db");
    let conn = db::open(&future).unwrap();
    conn.pragma_update(None, "user_version", db::SCHEMA_VERSION + 1)
        .unwrap();
    let before = fingerprint(&conn);
    assert_startup_rejected(&future, "does not match supported schema");
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        db::SCHEMA_VERSION + 1
    );
    assert_eq!(fingerprint(&conn), before);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM command_log", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let empty = dir.path().join("empty.db");
    let conn = Connection::open(&empty).unwrap();
    assert_startup_rejected(&empty, "does not match supported schema");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
