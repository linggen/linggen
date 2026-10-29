//! The SDK's background tasks onto the session task list
//! (doc/background-tasks-spec.md). The SDK owns them; this only mirrors what
//! it reports: the live set whenever it changes, and each task's progress and
//! end. A subagent task points at its pane tab.
use crate::engine::events::BackgroundTask;
use serde_json::Value;

fn text(event: &Value, key: &str) -> Option<String> {
    event[key].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// Apply one `task` / `tasks` frame. True when the list changed.
pub(super) fn apply(tasks: &crate::server::tasks::Tasks, session: &str, run: &str, event: &Value) -> bool {
    let start = |id: &str, kind: &str, title: &str| {
        let mut task = BackgroundTask::running(id, kind, title);
        if kind == "subagent" {
            task.subagent_id = Some(super::subagents::tracking_id(run, id));
        }
        task
    };
    if event["type"] == "tasks" {
        let live: Vec<&Value> = event["tasks"].as_array().into_iter().flatten().collect();
        let mut changed = false;
        for item in &live {
            let id = item["id"].as_str().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            match tasks.get(session, id) {
                Some(task) if task.is_running() => {}
                Some(_) => changed |= tasks.update(session, id, |task| {
                    task.status = "running".into();
                    task.ended_at = None;
                }),
                None => {
                    let kind = item["kind"].as_str().unwrap_or("task");
                    let title = item["title"].as_str().unwrap_or_default();
                    changed |= tasks.upsert(session, start(id, kind, title));
                }
            }
        }
        // Left the live set without a word of how: it ended. A notification
        // after this still names how.
        for task in tasks.list(session).into_iter().filter(BackgroundTask::is_running) {
            if !live.iter().any(|item| item["id"] == task.id.as_str()) {
                changed |= tasks.finish(session, &task.id, "completed", None);
            }
        }
        return changed;
    }
    let Some(id) = text(event, "id") else {
        return false;
    };
    if let Some(status) = text(event, "status") {
        let output = text(event, "output_file");
        return tasks.update(session, &id, |task| {
            task.end(&status);
            if let Some(summary) = text(event, "summary") {
                task.summary = Some(summary);
            }
            if output.is_some() {
                task.output_file = output;
            }
        });
    }
    if let Some(activity) = text(event, "activity") {
        return tasks.update(session, &id, |task| {
            if task.is_running() {
                task.activity = Some(activity);
            }
        });
    }
    if event["background"] == true && tasks.get(session, &id).is_none() {
        let kind = event["kind"].as_str().unwrap_or("task");
        let title = event["title"].as_str().unwrap_or_default();
        return tasks.upsert(session, start(&id, kind, title));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_live_set_lists_tasks_and_their_ends_follow() {
        let tasks = crate::server::tasks::Tasks::default();
        let snapshot = json!({"type":"tasks","tasks":[
            {"id":"t1","kind":"shell","title":"npm test"},
            {"id":"t2","kind":"subagent","title":"review"}]});
        assert!(apply(&tasks, "s", "run1", &snapshot));
        assert!(!apply(&tasks, "s", "run1", &snapshot), "the same set is no change");
        assert_eq!(tasks.get("s", "t2").unwrap().subagent_id.as_deref(), Some("sdk-sub-run1-t2"));
        assert!(apply(&tasks, "s", "run1", &json!({"type":"task","id":"t1","activity":"12 passed"})));
        assert_eq!(tasks.get("s", "t1").unwrap().activity.as_deref(), Some("12 passed"));
        let ended = json!({"type":"task","id":"t1","status":"failed","summary":"2 failed","output_file":"/tmp/o"});
        assert!(apply(&tasks, "s", "run1", &ended));
        let t1 = tasks.get("s", "t1").unwrap();
        assert_eq!((t1.status.as_str(), t1.summary.as_deref(), t1.activity), ("failed", Some("2 failed"), None));
        // t2 leaves the live set with no notification yet: ended.
        assert!(apply(&tasks, "s", "run1", &json!({"type":"tasks","tasks":[]})));
        assert_eq!(tasks.get("s", "t2").unwrap().status, "completed");
    }

    #[test]
    fn a_foreground_task_is_not_listed() {
        let tasks = crate::server::tasks::Tasks::default();
        let start = json!({"type":"task","id":"t3","kind":"subagent","title":"look","background":false});
        assert!(!apply(&tasks, "s", "run1", &start));
        assert!(tasks.list("s").is_empty());
    }
}
