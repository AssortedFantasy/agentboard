//! Cross-cutting requirements discovered by the independent scope audit.
use agentboard::db;
use std::sync::{Arc, Barrier};

#[test]
fn simultaneous_first_invocations_can_initialize_one_board() {
    // Every worker may be the first command from a separate harness. Opening a
    // new board must remain safe even before its WAL and schema exist.
    let dir = tempfile::tempdir().unwrap();
    for round in 0..5 {
        let path = Arc::new(dir.path().join(format!("first-{round}.db")));
        let barrier = Arc::new(Barrier::new(6));
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    db::open(path.as_path())
                        .map(|_| ())
                        .map_err(|error| format!("{error:#}"))
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(
            results.iter().all(Result::is_ok),
            "simultaneous initialization failed: {results:?}"
        );
    }
}
