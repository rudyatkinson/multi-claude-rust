//! `workflows.json`, the file of `src/main/workflows/store.ts`: the designed workflows, in a file
//! of their own. What the file holds beyond what this version knows is written back as it was.
//!
//! `WorkflowStore` kept the file in memory and wrote it after every change; here the holder keeps
//! the [`WorkflowsFile`], changes it with [`save_workflow`] / [`remove_workflow`] and writes it
//! with [`write_workflows_file`]. `existed` of the store is `path.exists()` before the first read.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};

use super::schema::{parse_workflow, parse_workflows};
use crate::error::{AppError, AppResult};
use crate::types::{Workflow, WorkflowsFile, WORKFLOW_LIMITS};

/// The keys the file has for this version; every other one is kept in `extra`.
const FILE_KEYS: [&str; 2] = ["version", "workflows"];

/// Reads `workflows.json`. A missing or broken file reads as no workflows; workflows that cannot
/// be read are left out ([`parse_workflows`]). `version` is the one this version writes, 1.
pub fn read_workflows_file(path: &Path) -> WorkflowsFile {
    let Ok(bytes) = fs::read(path) else { return WorkflowsFile::default() };
    let text = String::from_utf8_lossy(&bytes);
    // a file saved by an editor may start with a byte order mark
    let Ok(raw) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return WorkflowsFile::default() };
    let extra = raw
        .as_object()
        .map(|obj| obj.iter().filter(|(key, _)| !FILE_KEYS.contains(&key.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    WorkflowsFile { version: 1, workflows: parse_workflows(&raw), extra }
}

/// Writes `workflows.json` as `{ ...extra, version: 1, workflows }`, atomically.
pub fn write_workflows_file(path: &Path, file: &WorkflowsFile) -> AppResult<()> {
    let mut data: Map<String, Value> = file.extra.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    data.insert("version".into(), Value::from(1));
    data.insert("workflows".into(), serde_json::to_value(&file.workflows)?);
    write_json_atomic(path, &Value::Object(data))
}

/// Adds a workflow or replaces the one with its id. The value is unchecked: it is read as
/// [`parse_workflow`] reads it. Gives the workflow as it is kept.
pub fn save_workflow(file: &mut WorkflowsFile, raw: &Value) -> AppResult<Workflow> {
    let workflow = parse_workflow(raw).ok_or_else(|| AppError::invalid("Invalid workflow"))?;
    match file.workflows.iter_mut().find(|w| w.id == workflow.id) {
        Some(known) => *known = workflow.clone(),
        None => {
            if file.workflows.len() >= WORKFLOW_LIMITS.workflows {
                return Err(AppError::refused("There are too many workflows. Delete one first."));
            }
            file.workflows.push(workflow.clone());
        }
    }
    Ok(workflow)
}

/// Removes the workflow with that id. False when there was none (and nothing needs writing).
pub fn remove_workflow(file: &mut WorkflowsFile, id: &str) -> bool {
    let before = file.workflows.len();
    file.workflows.retain(|w| w.id != id);
    file.workflows.len() != before
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// `writeJsonAtomic`: `JSON.stringify(value, null, 2)` and a newline, written next to the file
/// and renamed over it. Windows can refuse the rename while another process has the target open;
/// it is retried once.
fn write_json_atomic(path: &Path, value: &Value) -> AppResult<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| AppError::io(format!("Could not create {}: {e}", parent.display())))?;
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}-{}", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text).map_err(|e| AppError::io(format!("Could not save {}: {e}", path.display())))?;
    if fs::rename(&tmp, path).is_err() {
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(AppError::io(format!("Could not save {}: {e}", path.display())));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn step(id: &str) -> Value {
        json!({ "id": id, "name": id, "profileId": "main", "prompt": format!("do {id}"), "gate": true })
    }
    fn raw_file(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn starts_empty_and_knows_that_nothing_was_there() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("workflows.json");
        assert!(!file.exists());
        assert!(read_workflows_file(&file).workflows.is_empty());
        assert!(!file.exists());
    }

    #[test]
    fn adds_replaces_and_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workflows.json");
        let mut file = read_workflows_file(&path);
        for raw in [
            json!({ "id": "a", "name": "A", "steps": [step("s1")] }),
            json!({ "id": "b", "name": "B", "steps": [] }),
            json!({ "id": "a", "name": "A2", "steps": [step("s1"), step("s2")] }),
        ] {
            save_workflow(&mut file, &raw).unwrap();
            write_workflows_file(&path, &file).unwrap();
        }
        assert_eq!(
            file.workflows.iter().map(|w| format!("{}:{}:{}", w.id, w.name, w.steps.len())).collect::<Vec<_>>(),
            ["a:A2:2", "b:B:0"]
        );
        assert!(remove_workflow(&mut file, "b"));
        write_workflows_file(&path, &file).unwrap();
        assert!(!remove_workflow(&mut file, "nope"));

        assert!(path.exists());
        let again = read_workflows_file(&path);
        assert_eq!(again.workflows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(again.workflows.iter().find(|w| w.id == "a").unwrap().steps[1].id, "s2");
        assert_eq!(raw_file(&path)["version"], json!(1));
    }

    #[test]
    fn refuses_what_is_not_a_workflow() {
        let mut file = WorkflowsFile::default();
        assert_eq!(save_workflow(&mut file, &json!({ "name": "no id" })).unwrap_err().to_string(), "Invalid workflow");
        assert_eq!(save_workflow(&mut file, &json!("text")).unwrap_err().to_string(), "Invalid workflow");
        assert!(file.workflows.is_empty());
    }

    #[test]
    fn refuses_a_workflow_beyond_the_limit_but_replaces_one_that_is_known() {
        let mut file = WorkflowsFile::default();
        for n in 0..WORKFLOW_LIMITS.workflows {
            save_workflow(&mut file, &json!({ "id": format!("w{n}"), "name": "W", "steps": [] })).unwrap();
        }
        let refused = save_workflow(&mut file, &json!({ "id": "more", "name": "W", "steps": [] })).unwrap_err();
        assert_eq!(refused.to_string(), "There are too many workflows. Delete one first.");
        assert_eq!(save_workflow(&mut file, &json!({ "id": "w0", "name": "W0", "steps": [] })).unwrap().name, "W0");
        assert_eq!(file.workflows.len(), WORKFLOW_LIMITS.workflows);
    }

    #[test]
    fn writes_back_what_a_later_version_put_into_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workflows.json");
        let later = json!({ "version": 2, "theme": "dark", "workflows": [{ "id": "a", "name": "A", "tags": ["x"], "steps": [{ "id": "s1", "retries": 3 }] }] });
        fs::write(&path, later.to_string()).unwrap();
        let mut file = read_workflows_file(&path);
        save_workflow(&mut file, &json!({ "id": "b", "name": "B", "steps": [] })).unwrap();
        write_workflows_file(&path, &file).unwrap();
        let written = raw_file(&path);
        assert_eq!(written["theme"], json!("dark"));
        assert_eq!(written["version"], json!(1));
        assert_eq!(written["workflows"][0]["id"], json!("a"));
        assert_eq!(written["workflows"][0]["tags"], json!(["x"]));
        assert_eq!(written["workflows"][0]["steps"][0]["id"], json!("s1"));
        assert_eq!(written["workflows"][0]["steps"][0]["retries"], json!(3));
    }

    #[test]
    fn reads_a_broken_file_as_empty_without_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workflows.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(path.exists());
        assert!(read_workflows_file(&path).workflows.is_empty());
    }
}
