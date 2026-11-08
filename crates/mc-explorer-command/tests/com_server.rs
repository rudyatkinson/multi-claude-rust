//! Loads the built DLL the way COM does, from an install-like folder, and talks to its class.
#![cfg(windows)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{FreeLibrary, CLASS_E_CLASSNOTAVAILABLE, E_FAIL, S_FALSE, S_OK};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, IClassFactory, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Ole::IObjectWithSite;
use windows::Win32::UI::Shell::{IExplorerCommand, IShellItem, IShellItemArray, SHCreateItemFromParsingName, SHCreateShellItemArrayFromShellItem, ECS_ENABLED, ECS_HIDDEN};
use windows_core::{s, IUnknown, Interface, GUID, HRESULT, HSTRING, PWSTR};

const CLSID: GUID = GUID::from_u128(0xE00837FA_9B7A_49E7_B33B_0115A2B2C52E);

type GetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type CanUnloadNow = unsafe extern "system" fn() -> HRESULT;

/// The DLL of this crate. `cargo test` compiles the library for the test harness but not the
/// `cdylib` artifact, so on a fresh target (CI) the DLL is built here first, with the same profile.
fn built_dll() -> PathBuf {
    let deps = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let profile_dir = deps.parent().unwrap().to_path_buf();
    let candidates = [deps.join("mc_explorer_command.dll"), profile_dir.join("mc_explorer_command.dll")];
    if let Some(found) = candidates.iter().find(|p| p.is_file()) {
        return found.clone();
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut build = std::process::Command::new(env!("CARGO"));
    build.arg("build").arg("--manifest-path").arg(&manifest).arg("--lib");
    if profile_dir.file_name().is_some_and(|name| name == "release") {
        build.arg("--release");
    }
    let status = build.status().expect("cargo runs");
    assert!(status.success(), "building the DLL failed");
    candidates.into_iter().find(|p| p.is_file()).expect("cargo built the DLL")
}

/// Takes a string the DLL allocated.
fn take(text: PWSTR) -> String {
    // SAFETY: the DLL returns a CoTaskMemAlloc'ed, NUL-terminated string.
    unsafe {
        let s = String::from_utf16_lossy(text.as_wide());
        CoTaskMemFree(Some(text.0 as *const c_void));
        s
    }
}

fn items(path: &Path) -> IShellItemArray {
    // SAFETY: plain shell calls.
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).unwrap();
        SHCreateShellItemArrayFromShellItem(&item).unwrap()
    }
}

#[test]
fn serves_the_menu_command() {
    // <install>\explorer\mc_explorer_command_x64.dll next to <install>\Multi Claude.exe
    let install = tempfile::tempdir().unwrap();
    let explorer = install.path().join("explorer");
    std::fs::create_dir(&explorer).unwrap();
    let dll = explorer.join("mc_explorer_command_x64.dll");
    std::fs::copy(built_dll(), &dll).unwrap();
    let exe = install.path().join("Multi Claude.exe");
    std::fs::write(&exe, b"").unwrap();
    let folder = install.path().join("my folder");
    std::fs::create_dir(&folder).unwrap();
    let file = install.path().join("notes.txt");
    std::fs::write(&file, b"x").unwrap();

    // SAFETY: the exports are called with valid pointers and their documented signatures.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        let module = LoadLibraryW(&HSTRING::from(dll.as_os_str())).unwrap();
        let get_class_object: GetClassObject = std::mem::transmute(GetProcAddress(module, s!("DllGetClassObject")).unwrap());
        let can_unload_now: CanUnloadNow = std::mem::transmute(GetProcAddress(module, s!("DllCanUnloadNow")).unwrap());
        assert_eq!(can_unload_now(), S_OK);

        let mut object = std::ptr::null_mut();
        assert_eq!(get_class_object(&IShellItem::IID, &IClassFactory::IID, &mut object), CLASS_E_CLASSNOTAVAILABLE);
        assert!(object.is_null());
        assert_eq!(get_class_object(&CLSID, &IClassFactory::IID, &mut object), S_OK);
        let factory = IClassFactory::from_raw(object);
        assert_eq!(can_unload_now(), S_FALSE);

        let command: IExplorerCommand = factory.CreateInstance(None).unwrap();
        assert_eq!(take(command.GetTitle(None).unwrap()), "Open in Multi Claude");
        assert_eq!(take(command.GetIcon(None).unwrap()).to_lowercase(), exe.to_string_lossy().to_lowercase());

        // a real folder shows the entry; a file, or nothing at all, hides it
        assert_eq!(command.GetState(&items(&folder), false).unwrap(), ECS_ENABLED.0 as u32);
        assert_eq!(command.GetState(&items(&file), false).unwrap(), ECS_HIDDEN.0 as u32);
        assert_eq!(command.GetState(None, false).unwrap(), ECS_HIDDEN.0 as u32);

        // no site set: the open folder cannot be asked for
        let with_site: IObjectWithSite = command.cast().unwrap();
        assert_eq!(with_site.GetSite::<IUnknown>().unwrap_err().code(), E_FAIL);
        with_site.SetSite(&factory).unwrap();
        assert!(with_site.GetSite::<IClassFactory>().is_ok());
        with_site.SetSite(None).unwrap();

        // without the app the entry is hidden everywhere
        std::fs::remove_file(&exe).unwrap();
        assert_eq!(command.GetState(&items(&folder), false).unwrap(), ECS_HIDDEN.0 as u32);
        assert!(command.GetIcon(None).is_err());

        factory.LockServer(true).unwrap();
        drop((command, with_site, factory));
        assert_eq!(can_unload_now(), S_FALSE, "a lock keeps the DLL");
        let mut object = std::ptr::null_mut();
        get_class_object(&CLSID, &IClassFactory::IID, &mut object).ok().unwrap();
        let factory = IClassFactory::from_raw(object);
        factory.LockServer(false).unwrap();
        drop(factory);
        assert_eq!(can_unload_now(), S_OK);

        FreeLibrary(module).unwrap();
        CoUninitialize();
    }
}
