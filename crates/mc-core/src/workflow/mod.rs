//! Workflows: schema logic, prompt variables, validation, preflight, the run reducer and the
//! files of a run.
//!
//! - [`schema`]: `src/shared/workflows.ts` (reading, prompt variables, editing, checks);
//! - [`engine`]: `src/main/workflows/engine.ts`, the pure reducer of a run, and what the
//!   I/O half (`WorkflowRuns`, in the Tauri crate) has to do with its effects;
//! - [`launcher`]: the pure parts of `src/main/workflows/launcher.ts` (file names, the first
//!   message, prompt values, the prompt and output files, pruning old run folders);
//! - [`store`]: the file of `src/main/workflows/store.ts` (`workflows.json`);
//! - [`transcript`]: `src/main/workflows/transcript.ts`, the last answer of a conversation file.
//!
//! The types (`Workflow`, `WorkflowStep`, `WorkflowRun`, ...) are in [`crate::types::workflows`].

pub mod engine;
pub mod launcher;
pub mod schema;
pub mod store;
pub mod transcript;

pub use engine::{
    new_run, reduce, view_of, Effect, Notice, Reduced, RunData, RunEvent, RunSession, StepData, Thread, CLOSED, ENDED,
    KEEP_FINISHED, RESTARTED, SILENT_MS,
};
pub use launcher::{
    check_run_folder, launch_message, output_file_name, prompt_file_name, prompt_values, prune_run_folders, run_folder,
    step_file_stem, write_step_output, write_step_prompt, KEEP_RUN_FILES_MS,
};
pub use schema::{
    free_step_name, free_step_name_from, is_active_run, is_model, move_step, parse_step, parse_workflow, parse_workflows,
    preflight, prompt_refs, ref_of, remove_step, rename_step, rename_step_refs, render_prompt, sample_workflow, thread_of,
    validate_workflow, NamedAccount, PromptValues, StepOutput,
};
pub use store::{read_workflows_file, remove_workflow, save_workflow, write_workflows_file};
pub use transcript::{last_assistant_text, read_last_answer, TAIL_BYTES};
