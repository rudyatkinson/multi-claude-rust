//! `limit-events.json` (`src/main/limitLog.ts`): a short record of failed turns and of the ends
//! of Claude Code's waits for a usage limit. It is there to be looked at: how a usage limit really
//! arrives can only be seen when an account runs into one. The entries themselves are made by
//! [`mc_core::hooks::limit_event_of`].

use std::path::{Path, PathBuf};

use mc_core::hooks::limit_event_of;
use mc_core::jsstr::cut16;
use mc_core::types::{HookEvent, LimitEvent, LIMIT_LOG_KEEP, LIMIT_LOG_TEXT};
use serde_json::Value;

pub const LIMIT_EVENTS_FILE: &str = "limit-events.json";

/// `<root>/limit-events.json`.
pub fn limit_events_file(root: &Path) -> PathBuf {
    root.join(LIMIT_EVENTS_FILE)
}

fn cut(text: &mut Option<String>) {
    if let Some(t) = text {
        let kept = cut16(t, LIMIT_LOG_TEXT).len();
        t.truncate(kept);
    }
}

/// Appends an entry to `<root>/limit-events.json`, keeping the last [`LIMIT_LOG_KEEP`]. What the
/// file held is kept as it was, whatever its entries look like; a file that is not a list is
/// started afresh. Failures are logged: the record is a convenience.
pub fn record_limit_event(root: &Path, entry: &LimitEvent) {
    let file = limit_events_file(root);
    let mut entry = entry.clone();
    for text in [&mut entry.error, &mut entry.details, &mut entry.r#type, &mut entry.text] {
        cut(text);
    }
    let known = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let mut list = match known {
        Some(Value::Array(list)) => list,
        _ => Vec::new(),
    };
    match serde_json::to_value(&entry) {
        Ok(value) => list.push(value),
        Err(e) => {
            log::warn!("limit-events.json: {e}");
            return;
        }
    }
    let start = list.len().saturating_sub(LIMIT_LOG_KEEP);
    if let Err(e) = write_json_atomic(&file, &Value::Array(list.split_off(start))) {
        log::warn!("limit-events.json: {e}");
    }
}

/// What `onHook` of the Electron main process did first with every event: notes it when it is a
/// failed turn or the end of a wait for a limit. `account` is the name of the session's profile
/// (`""` when it is not known).
pub fn record_hook(root: &Path, event: HookEvent, body: &Value, account: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    if let Some(entry) = limit_event_of(event, body, account, now) {
        record_limit_event(root, &entry);
    }
}

/// `writeJsonAtomic`: two-space JSON and a newline, written next to the file and renamed over it.
fn write_json_atomic(file: &Path, data: &Value) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let text = serde_json::to_string_pretty(data).map_err(std::io::Error::other)? + "\n";
    std::fs::write(&tmp, text)?;
    // Windows can refuse the rename while another process has the target open; retry once.
    std::fs::rename(&tmp, file).or_else(|_| std::fs::rename(&tmp, file)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn entry(at: &str) -> LimitEvent {
        LimitEvent {
            at: at.into(),
            account: "Main".into(),
            event: HookEvent::StopFailure,
            error: None,
            details: None,
            r#type: None,
            text: None,
        }
    }

    fn read(root: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(limit_events_file(root)).unwrap()).unwrap()
    }

    #[test]
    fn keeps_the_last_twenty() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..25 {
            record_limit_event(dir.path(), &entry(&i.to_string()));
        }
        let kept = read(dir.path());
        let kept = kept.as_array().unwrap();
        assert_eq!(kept.len(), 20);
        assert_eq!(kept[0]["at"], "5");
        assert_eq!(kept[19]["at"], "24");
    }

    #[test]
    fn starts_afresh_over_a_file_that_is_not_a_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(limit_events_file(dir.path()), r#"{"not":"a list"}"#).unwrap();
        record_limit_event(dir.path(), &entry("a"));
        assert_eq!(read(dir.path()), json!([{ "at": "a", "account": "Main", "event": "StopFailure" }]));
        std::fs::write(limit_events_file(dir.path()), "not json").unwrap();
        record_limit_event(dir.path(), &entry("b"));
        assert_eq!(read(dir.path()), json!([{ "at": "b", "account": "Main", "event": "StopFailure" }]));
    }

    #[test]
    fn keeps_what_the_file_held_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(limit_events_file(dir.path()), r#"[{"odd":true}, 5]"#).unwrap();
        record_limit_event(dir.path(), &entry("c"));
        let text = std::fs::read_to_string(limit_events_file(dir.path())).unwrap();
        assert!(text.ends_with("]\n"));
        assert!(text.contains("\n  {"), "two-space indent: {text}");
        assert_eq!(read(dir.path()), json!([{ "odd": true }, 5, { "at": "c", "account": "Main", "event": "StopFailure" }]));
    }

    #[test]
    fn cuts_the_texts_of_an_entry() {
        let dir = tempfile::tempdir().unwrap();
        record_limit_event(dir.path(), &LimitEvent { text: Some("é".repeat(400)), ..entry("d") });
        assert_eq!(read(dir.path())[0]["text"].as_str().unwrap().chars().count(), 300);
    }

    #[test]
    fn notes_only_the_hooks_that_tell_of_a_limit() {
        let dir = tempfile::tempdir().unwrap();
        record_hook(dir.path(), HookEvent::Stop, &json!({ "last_assistant_message": "done" }), "Main");
        assert!(!limit_events_file(dir.path()).exists());
        record_hook(dir.path(), HookEvent::StopFailure, &json!({ "error": "rate_limit" }), "Main");
        record_hook(dir.path(), HookEvent::Notification, &json!({ "notification_type": "quota_auto_resume_fired" }), "Work");
        let kept = read(dir.path());
        assert_eq!(kept[0]["error"], "rate_limit");
        assert_eq!(kept[0]["account"], "Main");
        assert_eq!(kept[1]["type"], "quota_auto_resume_fired");
        assert!(kept[1]["at"].as_str().unwrap().ends_with('Z'));
        // nothing is left behind next to the file
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, [LIMIT_EVENTS_FILE]);
    }
}
