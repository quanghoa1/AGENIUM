use agenium_eval::{run_task, tasks, Mode, Task};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scripted_golden_tasks_pass() {
    for t in tasks() {
        let r = run_task(&t, &Mode::Scripted).await.unwrap();
        assert!(r.passed, "{}: {}", r.id, r.detail);
        assert_eq!((r.runs, r.tool_errors, r.reopened), (2, 0, 0), "{}", r.id);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_solution_is_graded_as_failed() {
    let mut t: Task = tasks().into_iter().find(|t| t.id == "csv-total").unwrap();
    t.solution = || {
        vec![(
            "write_file",
            json!({"path": "deliverables/total.txt", "content": "499.0\n"}),
        )]
    };
    let r = run_task(&t, &Mode::Scripted).await.unwrap();
    assert!(!r.passed && r.closed, "{r:?}");
    assert!(r.detail.contains("expected 500.0"));
}
