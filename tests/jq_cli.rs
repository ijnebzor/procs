use serde_json::{Value, json};
use std::process::{Command, Output};

fn run_procsuf(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_procsuf"))
        .args([
            "--use-config",
            "default",
            "--only",
            "pid",
            "--interval",
            "0",
        ])
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json_stream(output: &Output) -> Vec<Value> {
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter::<Value>()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn jq_implies_canonical_json_and_transforms_the_complete_result() {
    let output = run_procsuf(&["--jq", "{pids: map(.pid), count: length}"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let pids = value["pids"].as_array().unwrap();
    assert_eq!(value["count"], json!(pids.len()));
    assert!(pids.iter().all(Value::is_number));
}

#[test]
fn jq_runs_after_per_record_where_filtering() {
    let output = run_procsuf(&["--where", "false", "--jq", "length"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(json_stream(&output), [json!(0)]);
}

#[test]
fn jq_emits_arrays_objects_and_scalars_as_an_ordered_json_stream() {
    let output = run_procsuf(&["--jq", r#"[1, 2], {kind: "object"}, "text", 3, true, null"#]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        json_stream(&output),
        [
            json!([1, 2]),
            json!({"kind": "object"}),
            json!("text"),
            json!(3),
            json!(true),
            json!(null),
        ]
    );
}

#[test]
fn jq_empty_stream_writes_nothing() {
    let output = run_procsuf(&["--jq", "empty"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
}

#[test]
fn jq_syntax_and_runtime_errors_exit_with_diagnostics() {
    let syntax = run_procsuf(&["--jq", "map("]);
    assert_eq!(syntax.status.code(), Some(1));
    assert!(stderr(&syntax).contains("invalid --jq expression"));

    let runtime = run_procsuf(&["--jq", "1 | .name"]);
    assert_eq!(runtime.status.code(), Some(1));
    assert!(stderr(&runtime).contains("failed to evaluate --jq expression"));
    assert!(stderr(&runtime).contains("cannot index"));
}

#[test]
fn jq_rejects_watch_and_jsonl_modes() {
    let watch = run_procsuf(&["--jq", ".", "--watch"]);
    assert_eq!(watch.status.code(), Some(1));
    assert!(stderr(&watch).contains("--jq cannot be combined with watch mode"));

    let jsonl = run_procsuf(&["--jq", ".", "--format", "jsonl"]);
    assert_eq!(jsonl.status.code(), Some(1));
    assert!(stderr(&jsonl).contains("--jq cannot be combined with --format jsonl"));
}
