//! The designed workflows, in a file of their own (`WorkflowStore` of
//! `src/main/workflows/store.ts`). The file's format is in `mc_core::workflow::store`; this keeps
//! it in memory and writes it after every change.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use mc_core::error::AppResult;
use mc_core::types::{Profile, Workflow, WorkflowsFile};
use mc_core::workflow::{read_workflows_file, remove_workflow, sample_workflow, save_workflow, write_workflows_file};
use serde_json::Value;

pub struct WorkflowStore {
    file: PathBuf,
    data: Mutex<WorkflowsFile>,
    /// False when the file was not there: nothing was ever designed on this computer.
    existed: bool,
}

impl WorkflowStore {
    /// Reads `file`; a missing or broken file reads as no workflows.
    pub fn new(file: PathBuf) -> Self {
        let existed = file.exists();
        let data = Mutex::new(read_workflows_file(&file));
        WorkflowStore { file, data, existed }
    }

    fn data(&self) -> MutexGuard<'_, WorkflowsFile> {
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn existed(&self) -> bool {
        self.existed
    }

    pub fn list(&self) -> Vec<Workflow> {
        self.data().workflows.clone()
    }

    pub fn get(&self, id: &str) -> Option<Workflow> {
        self.data().workflows.iter().find(|w| w.id == id).cloned()
    }

    /// Adds a workflow or replaces the one with its id. The value is unchecked; what cannot be
    /// read as a workflow is `Invalid workflow`. The list only changes once the file is written.
    pub fn save(&self, raw: &Value) -> AppResult<Workflow> {
        let mut data = self.data();
        let mut next = data.clone();
        let workflow = save_workflow(&mut next, raw)?;
        write_workflows_file(&self.file, &next)?;
        *data = next;
        Ok(workflow)
    }

    /// Removes the workflow with that id; nothing happens for one that is not there.
    pub fn remove(&self, id: &str) -> AppResult<()> {
        let mut data = self.data();
        let mut next = data.clone();
        if !remove_workflow(&mut next, id) {
            return Ok(());
        }
        write_workflows_file(&self.file, &next)?;
        *data = next;
        Ok(())
    }

    /// `seedWorkflows`: someone who opens the workflows for the first time finds one to look at
    /// and to change: plan on the main account, implement on another one. True when it was written.
    pub fn seed(&self, accounts: &[Profile]) -> bool {
        if self.existed || !self.data().workflows.is_empty() {
            return false;
        }
        let Some(first) = accounts.iter().find(|p| p.is_default).or_else(|| accounts.first()) else { return false };
        let second = accounts.iter().find(|p| p.id != first.id).unwrap_or(first);
        let sample = sample_workflow(|| uuid::Uuid::new_v4().to_string(), &first.id, &second.id);
        let saved = serde_json::to_value(&sample).map_err(Into::into).and_then(|raw| self.save(&raw));
        match saved {
            Ok(_) => true,
            Err(e) => {
                // the list simply starts empty
                log::warn!("workflows: the sample could not be saved: {e}");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn profile(id: &str, is_default: bool) -> Profile {
        serde_json::from_value(json!({
            "id": id, "name": id, "slug": id, "color": "#fff", "isDefault": is_default, "configDir": null,
            "share": { "dirs": [], "files": [], "localSettingsKeys": [] }
        }))
        .unwrap()
    }

    #[test]
    fn seeds_the_sample_once_and_only_where_nothing_was_designed() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("workflows.json");
        let store = WorkflowStore::new(file.clone());
        assert!(!store.existed());
        assert!(store.seed(&[profile("work", false), profile("main", true)]));
        let seeded = store.list();
        assert_eq!(seeded.len(), 1);
        let accounts: Vec<&str> = seeded[0].steps.iter().map(|s| s.profile_id.as_str()).collect();
        assert_eq!(accounts, ["main", "work", "main"]);
        assert!(!store.seed(&[profile("main", true)]));
        assert_eq!(WorkflowStore::new(file.clone()).list(), seeded);

        // a file that is there, even an empty one, was the user's choice
        let emptied = WorkflowStore::new(file);
        emptied.remove(&seeded[0].id).unwrap();
        let again = WorkflowStore::new(tmp.path().join("workflows.json"));
        assert!(again.existed());
        assert!(!again.seed(&[profile("main", true)]));
        assert!(again.list().is_empty());
    }

    #[test]
    fn keeps_the_list_as_it_was_when_the_file_cannot_be_written() {
        let tmp = tempfile::tempdir().unwrap();
        // a folder where the file should be
        let file = tmp.path().join("workflows.json");
        std::fs::create_dir_all(file.join("x")).unwrap();
        let store = WorkflowStore::new(file);
        let raw = json!({ "id": "a", "name": "A", "steps": [] });
        assert!(store.save(&raw).is_err());
        assert!(store.list().is_empty());
        assert_eq!(store.save(&json!({ "nope": 1 })).unwrap_err().to_string(), "Invalid workflow");
    }
}
