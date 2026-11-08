//! The command surface. Naming rule: Electron channel `a:b` becomes Tauri command `a_b`
//! (`profiles:create` → `profiles_create`), so `src/api.ts` in the renderer is a mechanical map.
//! Every command validates its arguments with `mc_core::validate` before touching a service and
//! returns `CmdResult<T>`; the error text is what the user reads.

pub mod app;

/// All commands registered with the Tauri builder. Feature packages add their modules here.
pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![app::app_get_version]
}
