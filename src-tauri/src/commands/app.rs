use tauri::AppHandle;

/// The application's own version, shown next to Claude Code's in the sidebar footer.
#[tauri::command]
pub fn app_get_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}
