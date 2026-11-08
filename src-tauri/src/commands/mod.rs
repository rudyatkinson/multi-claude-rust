//! The command surface. Naming rule: Electron channel `a:b` becomes Tauri command `a_b` in
//! snake_case (`profiles:create` → `profiles_create`, `app:getState` → `app_get_state`,
//! `skills:pickImport` → `skills_pick_import`); `src/api.ts` in the renderer does the same mapping.
//! Argument names are the camelCase keys listed in `INVOKE_PARAMS` of `src/api.ts`; Tauri maps them
//! onto snake_case Rust parameters (`baseHash` → `base_hash`, `ref` → `r#ref`). Send channels take a
//! single `payload` argument. Every command validates with `mc_core::validate` before touching a
//! service and returns `CmdResult<T>`; the error text is what the user reads.

pub mod app;
pub mod attention;
pub mod layout;
pub mod mcp;
pub mod profiles;
pub mod sessions;
pub mod usage;

/// All commands registered with the Tauri builder. Package commands are added here at integration.
pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![app::app_get_version]
}
