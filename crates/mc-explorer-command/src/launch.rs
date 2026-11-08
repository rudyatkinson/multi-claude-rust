//! Starting the app: where its executable is, its command line, and the process.

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, E_FAIL, HANDLE, HMODULE};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, OpenProcess, UpdateProcThreadAttribute, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_CREATE_PROCESS, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, STARTUPINFOEXW, STARTUPINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, GetShellWindow, GetWindowThreadProcessId, ASFW_ANY};
use windows_core::{PCWSTR, PWSTR};

use crate::server::OBJECTS;

/// The DLL ships in `<install>\explorer` (Tauri puts resources next to the executable), one folder
/// below the app. The executable keeps Cargo's binary name unless `tauri.conf.json` sets
/// `mainBinaryName`, so both names are tried.
const APP_EXE_NAMES: [&str; 2] = ["Multi Claude.exe", "multi-claude.exe"];
const OPEN_SWITCH: &str = "--open-folder=";
/// The longest path Windows has.
const MAX_LONG_PATH: usize = 32_768;

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;

/// A NUL-terminated wide string.
pub(crate) fn wide_nul(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// The full path of this DLL.
fn module_path() -> Option<PathBuf> {
    let mut module = HMODULE::default();
    // any address inside the image names the module; UNCHANGED_REFCOUNT takes no reference
    let inside = std::ptr::addr_of!(OBJECTS).cast::<u16>();
    // SAFETY: `inside` points into this DLL, and `module` is a valid out pointer.
    unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(inside), &mut module) }.ok()?;
    let mut buffer = vec![0u16; 1024];
    loop {
        // SAFETY: the buffer is writable for its whole length.
        let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
        if len == 0 {
            return None;
        }
        if len < buffer.len() {
            buffer.truncate(len);
            return Some(PathBuf::from(OsString::from_wide(&buffer)));
        }
        // the name was cut off
        if buffer.len() >= MAX_LONG_PATH {
            return None;
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

/// The app's executable, when it is there.
pub(crate) fn app_exe() -> Option<PathBuf> {
    let dll = module_path()?;
    let install = dll.parent()?.parent()?;
    APP_EXE_NAMES.iter().map(|name| install.join(name)).find(|exe| exe.is_file())
}

/// Appends `arg` as one quoted argument following the `CommandLineToArgvW` rules, so that a path
/// ending in a backslash (`C:\`) does not swallow the closing quote: backslashes are doubled
/// before a quote (theirs or the closing one), and an embedded quote is escaped.
pub(crate) fn append_quoted(line: &mut Vec<u16>, arg: &[u16]) {
    let backslashes = |line: &mut Vec<u16>, count: usize| line.extend(std::iter::repeat_n(BACKSLASH, count));
    line.push(QUOTE);
    let mut pending = 0;
    for &c in arg {
        if c == BACKSLASH {
            pending += 1;
            continue;
        }
        backslashes(line, if c == QUOTE { pending * 2 + 1 } else { pending });
        pending = 0;
        line.push(c);
    }
    backslashes(line, pending * 2);
    line.push(QUOTE);
}

/// `"<exe>" "--open-folder=<folder>"`, NUL-terminated.
pub(crate) fn command_line(exe: &[u16], folder: &[u16]) -> Vec<u16> {
    let mut arg: Vec<u16> = OPEN_SWITCH.encode_utf16().collect();
    arg.extend_from_slice(folder);
    let mut line = Vec::with_capacity(exe.len() + 2 * arg.len() + 8);
    line.push(QUOTE);
    line.extend_from_slice(exe);
    line.push(QUOTE);
    line.push(SPACE);
    append_quoted(&mut line, &arg);
    line.push(0);
    line
}

/// Closes a handle when it goes out of scope.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle is open and owned here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn close_process(process: &PROCESS_INFORMATION) {
    drop(OwnedHandle(process.hThread));
    drop(OwnedHandle(process.hProcess));
}

/// Starts the app as a child of Explorer. This surrogate runs with the package's identity, and a
/// direct child would inherit it: removing the package could then take the running app down.
fn start_as_explorer_child(exe: PCWSTR, mut command_line: Vec<u16>, directory: PCWSTR) -> bool {
    // SAFETY: Win32 calls with valid arguments; the attribute list lives in `buffer`, is deleted
    // before the buffer goes, and the parent handle it points to outlives CreateProcessW.
    unsafe {
        let shell = GetShellWindow();
        if shell.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        if GetWindowThreadProcessId(shell, Some(&mut pid)) == 0 || pid == 0 {
            return false;
        }
        let Ok(parent) = OpenProcess(PROCESS_CREATE_PROCESS, false, pid) else {
            return false;
        };
        let parent = OwnedHandle(parent);

        let mut size = 0usize;
        // fails with ERROR_INSUFFICIENT_BUFFER and gives the size
        let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        if size == 0 {
            return false;
        }
        // usize elements keep the list pointer-aligned
        let mut buffer = vec![0usize; size.div_ceil(size_of::<usize>())];
        let attributes = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast::<c_void>());
        if InitializeProcThreadAttributeList(Some(attributes), 1, None, &mut size).is_err() {
            return false;
        }
        let mut started = false;
        let value = std::ptr::addr_of!(parent.0).cast::<c_void>();
        if UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize, Some(value), size_of::<HANDLE>(), None, None).is_ok() {
            let mut startup = STARTUPINFOEXW::default();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = attributes;
            let mut process = PROCESS_INFORMATION::default();
            let flags = CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
            let created = CreateProcessW(
                exe,
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                None,
                directory,
                &startup.StartupInfo,
                &mut process,
            );
            if created.is_ok() {
                started = true;
                close_process(&process);
            }
        }
        DeleteProcThreadAttributeList(attributes);
        started
    }
}

fn start_directly(exe: PCWSTR, mut command_line: Vec<u16>, directory: PCWSTR) -> windows_core::Result<()> {
    let startup = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: the strings are NUL-terminated and outlive the call; the command line is writable.
    unsafe {
        CreateProcessW(exe, Some(PWSTR(command_line.as_mut_ptr())), None, None, false, PROCESS_CREATION_FLAGS(0), None, directory, &startup, &mut process)?;
    }
    close_process(&process);
    Ok(())
}

/// Runs `"<exe>" "--open-folder=<folder>"`.
pub(crate) fn launch_app(folder: &[u16]) -> windows_core::Result<()> {
    let exe_path = app_exe().ok_or(E_FAIL)?;
    let exe: Vec<u16> = exe_path.as_os_str().encode_wide().collect();
    let exe_z = wide_nul(exe_path.as_os_str());
    let directory = wide_nul(exe_path.parent().map(Path::as_os_str).unwrap_or_default());
    let line = command_line(&exe, folder);

    // lets the app (or its already running instance) come to the front
    // SAFETY: no arguments to get wrong.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let (exe, directory) = (PCWSTR(exe_z.as_ptr()), PCWSTR(directory.as_ptr()));
    if start_as_explorer_child(exe, line.clone(), directory) {
        return Ok(());
    }
    start_directly(exe, line, directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::UI::Shell::CommandLineToArgvW;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// What a started program gets as its arguments.
    fn argv(line: &[u16]) -> Vec<String> {
        let mut count = 0i32;
        // SAFETY: `line` is NUL-terminated; the array is freed with LocalFree as documented.
        unsafe {
            let list = CommandLineToArgvW(PCWSTR(line.as_ptr()), &mut count);
            assert!(!list.is_null());
            let args = (0..count as usize).map(|i| String::from_utf16_lossy((*list.add(i)).as_wide())).collect();
            LocalFree(Some(HLOCAL(list.cast())));
            args
        }
    }

    #[test]
    fn quotes_like_the_cpp_version() {
        let quoted = |arg: &str| {
            let mut line = Vec::new();
            append_quoted(&mut line, &wide(arg));
            String::from_utf16_lossy(&line)
        };
        assert_eq!(quoted(r"D:\code\my app"), r#""D:\code\my app""#);
        assert_eq!(quoted(r"C:\"), r#""C:\\""#);
        assert_eq!(quoted(r"\\server\share\"), r#""\\server\share\\""#);
        assert_eq!(quoted(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quoted(""), r#""""#);
    }

    #[test]
    fn hands_the_folder_over_as_one_argument() {
        let exe = r"C:\Users\me\AppData\Local\Multi Claude\multi-claude.exe";
        for folder in [r"C:\", r"D:\code\my app", r"D:\code\trailing\", r"\\server\share\x y\", r"D:\100% done\ünïcode", r#"D:\odd"quote"#] {
            let line = command_line(&wide(exe), &wide(folder));
            assert_eq!(line.last(), Some(&0));
            assert_eq!(argv(&line), [exe.to_owned(), format!("--open-folder={folder}")], "{folder}");
        }
    }

    #[test]
    fn knows_the_file_it_was_loaded_from() {
        // in a test that is the test executable, whose image holds the statics
        assert!(module_path().is_some_and(|p| p.is_file()));
    }
}
