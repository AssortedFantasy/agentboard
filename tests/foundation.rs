use agentboard::{db,model::{Output,Receipt},render};
use serde_json::json;

#[test]
fn board_reopens_and_rejects_future_schema() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("board.db");
    let conn=db::open(&path).unwrap();
    assert_eq!(db::get_object(&conn,1).unwrap()["path"],"/");
    drop(conn);
    let conn=db::open(&path).unwrap();
    conn.pragma_update(None,"user_version",999).unwrap(); drop(conn);
    assert!(db::open(path).unwrap_err().to_string().contains("newer"));
}

#[test]
fn output_budget_keeps_json_valid_and_does_not_consume_truncated_content() {
    let mut output=Output::one("objects",json!({"id":2,"kind":"post","revision":1,"title":"hello","body":"🦀".repeat(10000),"notification_id":3}));
    output.receipts.push(Receipt {object_id:2,revision:1,full:true}); output.notification_ids.push(3);
    for format in ["text","json","jsonl"] {
        let rendered=render::render(&output,format,false,2048).unwrap();
        assert!(rendered.text.len()<=2048); assert!(rendered.receipts.is_empty()); assert!(rendered.notification_ids.is_empty());
        if format=="json" { let v:serde_json::Value=serde_json::from_str(&rendered.text).unwrap(); assert_eq!(v["more"],true); }
        if format=="jsonl" { for line in rendered.text.lines() { serde_json::from_str::<serde_json::Value>(line).unwrap(); } }
    }
}

#[test]
fn complete_content_preserves_raw_body_and_receipt() {
    let mut output=Output::one("objects",json!({"id":2,"kind":"post","revision":1,"title":"hello","body":"first\n\"second\" \\ third"}));
    output.receipts.push(Receipt {object_id:2,revision:1,full:true});
    let rendered=render::render(&output,"text",false,4096).unwrap();
    assert!(rendered.text.contains("first\n\"second\" \\ third")); assert!(rendered.receipts[0].full);
    assert!(!render::render(&output,"text",true,4096).unwrap().receipts[0].full);
}
