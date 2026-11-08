//! The tray icon (`createTray`, `trayMenu`, `applyTraySetting` of `src/main/index.ts`): a click
//! brings the window to the front; its menu opens the window, starts a session, shows each
//! account's usage and quits; its tooltip counts the running sessions.
//!
//! A Tauri menu is built before it opens, not as it opens, so the menu is built again whenever the
//! pointer comes onto the icon (`Enter`), which happens before any click can open it. It is never
//! replaced on the click itself: Windows shows the menu while the click is still being handled,
//! and replacing a menu that is open closes it. Linux reports no pointer events and shows the menu
//! it was given, so there the menu is built again with the session list, at most every few seconds.
//!
//! Everything that touches the icon runs on the main thread, which owns the icon's window.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mc_core::tray_model::{tray_account_lines, tray_tooltip};
use mc_core::types::{NewSession, SessionKind, SessionStatus};
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::app::window::{self, bring_to_front, main_window};
use crate::app::{now_ms, state as app_state};
use crate::events;
use crate::state::{lock, AppState};

/// The id of the one tray icon.
pub const ID: &str = "main";

const OPEN: &str = "tray:open";
const NEW_SESSION: &str = "tray:newSession";
const QUIT: &str = "tray:quit";

/// The tooltip the icon shows; None while there is no icon.
static TOOLTIP: Mutex<Option<String>> = Mutex::new(None);

/// The icon file, the first that exists: shipped with the app, or in `src-tauri/icons` for a
/// development run. macOS takes the template image at twice its size, which the menu bar draws at
/// 18 points.
#[cfg(windows)]
const ICON_FILES: &[&str] = &["icon.ico"];
#[cfg(target_os = "macos")]
const ICON_FILES: &[&str] = &["tray@2x.png", "tray.png"];
#[cfg(not(any(windows, target_os = "macos")))]
const ICON_FILES: &[&str] = &["icon.png"];

/// Whether the tray icon is there.
pub fn exists(app: &AppHandle) -> bool {
    app.tray_by_id(ID).is_some()
}

/// `applyTraySetting`: the icon follows `settings.tray` (on unless false). Without it a hidden
/// window could not be brought back, so turning it off shows the window.
pub fn apply_setting(app: &AppHandle) {
    let handle = app.clone();
    let posted = app.run_on_main_thread(move || {
        let Some(state) = app_state(&handle) else { return };
        if state.settings().tray != Some(false) {
            create(&handle);
            return;
        }
        remove(&handle);
        if main_window(&handle).is_some_and(|w| !w.is_visible().unwrap_or(true)) {
            window::show(&state);
        }
    });
    if let Err(e) = posted {
        log::warn!("tray: {e}");
    }
}

/// `createTray`: makes the icon unless it is there. False when it could not be made (no tray on
/// this system): closing the window then quits, as without the setting. Call it on the main thread.
pub fn create(app: &AppHandle) -> bool {
    if exists(app) {
        return true;
    }
    let Some(state) = app_state(app) else { return false };
    let Some(icon) = load_icon(app) else {
        log::warn!("tray: no icon to show");
        return false;
    };
    let menu = match build_menu(app, &state) {
        Ok(menu) => menu,
        Err(e) => {
            log::warn!("tray: the menu could not be built: {e}");
            return false;
        }
    };
    let tooltip = tooltip_of(&state);
    let builder = TrayIconBuilder::with_id(ID)
        .icon(icon)
        .tooltip(&tooltip)
        // Linux shows a menu only when one is set
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(on_tray_event);
    #[cfg(target_os = "macos")]
    let builder = builder.icon_as_template(true);
    match builder.build(app) {
        Ok(_) => {
            *lock(&TOOLTIP) = Some(tooltip);
            log::info!("tray: icon created");
            true
        }
        Err(e) => {
            log::warn!("tray: the icon could not be created: {e}");
            false
        }
    }
}

/// `destroyTray`: removes the icon, on the main thread, which alone can take its window down.
pub fn destroy(app: &AppHandle) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || remove(&handle)) {
        log::warn!("tray: {e}");
    }
}

fn remove(app: &AppHandle) {
    *lock(&TOOLTIP) = None;
    if let Some(tray) = app.remove_tray_by_id(ID) {
        // gone even if something else still holds the icon
        let _ = tray.set_visible(false);
        log::info!("tray: icon removed");
    }
}

/// The session list changed: the tooltip counts the running sessions again, and on Linux the menu
/// is built again. Does nothing when there is no icon or nothing it shows changed; never waits for
/// the main thread (the list may change while it waits for something else).
pub fn refresh(app: &AppHandle) {
    let Some(state) = app_state(app) else { return };
    let tooltip = tooltip_of(&state);
    let tooltip_changed = match lock(&TOOLTIP).as_mut() {
        None => return,
        Some(shown) if *shown == tooltip => false,
        Some(shown) => {
            shown.clone_from(&tooltip);
            true
        }
    };
    let menu_due = linux_menu_due();
    if !tooltip_changed && !menu_due {
        return;
    }
    let handle = app.clone();
    let posted = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(ID) else { return };
        if tooltip_changed {
            if let Err(e) = tray.set_tooltip(Some(&tooltip)) {
                log::warn!("tray tooltip: {e}");
            }
        }
        if menu_due {
            rebuild_menu(&handle, &tray);
        }
    });
    if let Err(e) = posted {
        log::warn!("tray: {e}");
    }
}

/// Linux: whether the menu was last built long enough ago to be built again.
#[cfg(not(any(windows, target_os = "macos")))]
fn linux_menu_due() -> bool {
    use std::sync::atomic::{AtomicI64, Ordering};
    const EVERY_MS: i64 = 5_000;
    static BUILT_AT: AtomicI64 = AtomicI64::new(0);
    let now = now_ms();
    let last = BUILT_AT.load(Ordering::SeqCst);
    now - last >= EVERY_MS && BUILT_AT.compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst).is_ok()
}

#[cfg(any(windows, target_os = "macos"))]
fn linux_menu_due() -> bool {
    false
}

/// `runningSessions`: the Claude sessions that are running.
fn running_sessions(state: &AppState) -> usize {
    state.sessions.list().iter().filter(|s| s.status == SessionStatus::Running && s.kind == SessionKind::Claude).count()
}

fn tooltip_of(state: &AppState) -> String {
    tray_tooltip(running_sessions(state))
}

/// `trayMenu`: what is true now. Call it on the main thread.
fn build_menu(app: &AppHandle, state: &AppState) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItemBuilder::with_id(OPEN, "Open Multi Claude").build(app)?)?;
    menu.append(&MenuItemBuilder::with_id(NEW_SESSION, "New session…").enabled(state.claude_path().is_some()).build(app)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let lines = tray_account_lines(&state.profiles.list(), &state.usage.snapshot(), now_ms());
    for (index, line) in lines.iter().enumerate() {
        menu.append(&MenuItemBuilder::with_id(format!("tray:account:{index}"), plain_label(line)).enabled(false).build(app)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItemBuilder::with_id(QUIT, "Quit").build(app)?)?;
    Ok(menu)
}

/// A menu label shown as it is: a single `&` would mark the next letter as the item's key.
fn plain_label(text: &str) -> String {
    text.replace('&', "&&")
}

fn rebuild_menu(app: &AppHandle, tray: &TrayIcon) {
    let Some(state) = app_state(app) else { return };
    match build_menu(app, &state) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                log::warn!("tray menu: {e}");
            }
        }
        Err(e) => log::warn!("tray menu: {e}"),
    }
}

/// A click brings the window to the front; the pointer on the icon builds the menu again.
fn on_tray_event(tray: &TrayIcon, event: TrayIconEvent) {
    let app = tray.app_handle();
    match event {
        TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
            if let Some(state) = app_state(app) {
                bring_to_front(&state);
            }
        }
        TrayIconEvent::Enter { .. } => rebuild_menu(app, tray),
        _ => {}
    }
}

/// The items of the tray menu. Registered once for the app (`AppHandle::on_menu_event`): a
/// handler given to the icon's builder would stay behind with every icon made.
pub fn on_menu_event(app: &AppHandle, event: &MenuEvent) {
    match event.id().as_ref() {
        OPEN => {
            if let Some(state) = app_state(app) {
                bring_to_front(&state);
            }
        }
        NEW_SESSION => {
            if let Some(state) = app_state(app) {
                bring_to_front(&state);
            }
            events::emit(app, events::UI_NEW_SESSION, NewSession {});
        }
        QUIT => crate::app::quit::request_quit(app),
        _ => {}
    }
}

// ---------- the icon ----------

fn load_icon(app: &AppHandle) -> Option<Image<'static>> {
    let from_file = ICON_FILES.iter().filter_map(|name| icon_path(app, name)).find_map(|path| match read_icon(app, &path) {
        Ok(icon) => Some(icon),
        Err(e) => {
            log::warn!("tray icon {}: {e}", path.display());
            None
        }
    });
    from_file.or_else(|| app.default_window_icon().map(|icon| icon.clone().to_owned()))
}

/// Where an icon file is: next to the app's resources, or in the sources for a development run.
fn icon_path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    let bundled = app.path().resource_dir().ok().map(|dir| dir.join(name));
    let sources = tauri::is_dev().then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("icons").join(name));
    bundled.into_iter().chain(sources).find(|path| path.is_file())
}

/// Windows: the entry of the `.ico` made for the notification area's size, which the shell would
/// otherwise shrink from the largest one.
#[cfg(windows)]
fn read_icon(app: &AppHandle, path: &Path) -> tauri::Result<Image<'static>> {
    let bytes = std::fs::read(path)?;
    let scale = main_window(app).and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
    let size = (16.0 * scale).round() as u32;
    Image::from_bytes(ico_png_entry(&bytes, size).unwrap_or(&bytes))
}

#[cfg(not(windows))]
fn read_icon(_app: &AppHandle, path: &Path) -> tauri::Result<Image<'static>> {
    Image::from_path(path)
}

/// The PNG of the `.ico` entry for `size` pixels: the smallest at least that large, else the
/// largest. None when the file has no PNG entries (or is no `.ico`).
#[cfg(any(windows, test))]
fn ico_png_entry(ico: &[u8], size: u32) -> Option<&[u8]> {
    let u16_at = |at: usize| ico.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| ico.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return None;
    }
    let entries: Vec<(u32, &[u8])> = (0..usize::from(u16_at(4)?))
        .filter_map(|index| {
            let at = 6 + 16 * index;
            // a width of 0 stands for 256
            let width = match *ico.get(at)? {
                0 => 256,
                w => u32::from(w),
            };
            let len = usize::try_from(u32_at(at + 8)?).ok()?;
            let offset = usize::try_from(u32_at(at + 12)?).ok()?;
            let data = ico.get(offset..offset.checked_add(len)?)?;
            data.starts_with(b"\x89PNG").then_some((width, data))
        })
        .collect();
    let fitting = entries.iter().filter(|(width, _)| *width >= size).min_by_key(|(width, _)| *width);
    fitting.or_else(|| entries.iter().max_by_key(|(width, _)| *width)).map(|(_, data)| *data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `.ico` whose entries hold `\x89PNG<width>` instead of images.
    fn ico(widths: &[u8]) -> Vec<u8> {
        let mut head = vec![0, 0, 1, 0];
        head.extend_from_slice(&(widths.len() as u16).to_le_bytes());
        let mut data = Vec::new();
        let start = 6 + 16 * widths.len();
        for width in widths {
            let body = [b"\x89PNG".as_slice(), width.to_string().as_bytes()].concat();
            head.extend_from_slice(&[*width, *width, 0, 0, 1, 0, 32, 0]);
            head.extend_from_slice(&(body.len() as u32).to_le_bytes());
            head.extend_from_slice(&((start + data.len()) as u32).to_le_bytes());
            data.extend_from_slice(&body);
        }
        [head, data].concat()
    }

    #[test]
    fn picks_the_ico_entry_for_the_size() {
        let file = ico(&[16, 32, 0]);
        assert_eq!(ico_png_entry(&file, 16), Some(b"\x89PNG16".as_slice()));
        // 20 pixels at 125%: the next larger entry
        assert_eq!(ico_png_entry(&file, 20), Some(b"\x89PNG32".as_slice()));
        // 0 is 256
        assert_eq!(ico_png_entry(&file, 48), Some(b"\x89PNG0".as_slice()));
        // larger than any: the largest
        assert_eq!(ico_png_entry(&ico(&[16, 32]), 64), Some(b"\x89PNG32".as_slice()));
        assert_eq!(ico_png_entry(b"\x89PNG not an ico", 16), None);
        assert_eq!(ico_png_entry(&file[..20], 16), None);
    }

    #[test]
    fn reads_the_shipped_icon_at_tray_sizes() {
        let file = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/icon.ico")).unwrap();
        for (size, expected) in [(16, 16), (20, 24), (24, 24), (32, 32), (40, 48)] {
            let image = Image::from_bytes(ico_png_entry(&file, size).unwrap()).unwrap();
            assert_eq!((image.width(), image.height()), (expected, expected), "for {size} pixels");
        }
    }

    #[test]
    fn menu_labels_show_ampersands() {
        assert_eq!(plain_label("Tom & Jerry — 5h 3%"), "Tom && Jerry — 5h 3%");
    }
}
