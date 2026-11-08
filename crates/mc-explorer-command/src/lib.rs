//! "Open in Multi Claude" in the Windows 11 File Explorer context menu: a COM in-process server
//! with one class, an `IExplorerCommand` that also takes a site (`IObjectWithSite`) to learn the
//! folder of the view hosting the menu. Port of `native/explorer-command/mc_explorer_command.cpp`
//! of the Electron app.
//!
//! COM loads the DLL into a surrogate (dllhost.exe) through the sparse package in
//! `src-tauri/resources/explorer`; `scripts/build-explorer-dll.ps1` (`npm run build:shell`) builds
//! it and copies it there as `mc_explorer_command_x64.dll`. It exports `DllGetClassObject` and
//! `DllCanUnloadNow` and nothing else. Elsewhere than on Windows this crate is an empty library.
//!
//! Nothing may unwind out of an entry point into Explorer: every method runs its body through
//! `guard`, which turns a panic into `E_UNEXPECTED`. The release profile aborts on panic, so
//! the code also stays clear of panicking paths (no indexing, no `unwrap`).
#![allow(clippy::missing_safety_doc)]

#[cfg(windows)]
mod command;
#[cfg(windows)]
mod launch;

#[cfg(windows)]
pub use server::*;

#[cfg(windows)]
mod server {
    use std::ffi::c_void;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_POINTER, E_UNEXPECTED, S_FALSE, S_OK};
    use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
    use windows_core::{implement, Interface, Ref, BOOL, GUID, HRESULT};

    use crate::command::Command;

    /// The class of the menu command. Keep in sync with `src-tauri/resources/explorer/AppxManifest.xml`
    /// and `src-tauri/src/explorer_menu/package.rs`; a test there compares them.
    pub const CLSID: GUID = GUID::from_u128(0xE00837FA_9B7A_49E7_B33B_0115A2B2C52E);

    /// Live objects (commands and class factories) and `LockServer` locks, for `DllCanUnloadNow`.
    pub(crate) static OBJECTS: AtomicIsize = AtomicIsize::new(0);
    static LOCKS: AtomicIsize = AtomicIsize::new(0);

    /// Held by every object this DLL hands out, so the DLL stays loaded while one is alive.
    pub(crate) struct ObjectCount;

    impl ObjectCount {
        pub(crate) fn new() -> Self {
            OBJECTS.fetch_add(1, Ordering::SeqCst);
            ObjectCount
        }
    }

    impl Drop for ObjectCount {
        fn drop(&mut self) {
            OBJECTS.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Runs the body of a COM method; a panic becomes `E_UNEXPECTED` instead of unwinding into
    /// the caller.
    pub(crate) fn guard<T>(body: impl FnOnce() -> windows_core::Result<T>) -> windows_core::Result<T> {
        catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| Err(E_UNEXPECTED.into()))
    }

    #[implement(IClassFactory)]
    struct Factory {
        _count: ObjectCount,
    }

    impl IClassFactory_Impl for Factory_Impl {
        fn CreateInstance(&self, outer: Ref<windows_core::IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> windows_core::Result<()> {
            guard(|| {
                if object.is_null() {
                    return Err(E_POINTER.into());
                }
                // SAFETY: COM hands a writable out pointer; it was checked for null above.
                unsafe { *object = std::ptr::null_mut() };
                if !outer.is_null() {
                    return Err(CLASS_E_NOAGGREGATION.into());
                }
                if iid.is_null() {
                    return Err(E_INVALIDARG.into());
                }
                let command: windows_core::IUnknown = Command::new().into();
                // SAFETY: `iid` and `object` are valid, as checked above.
                unsafe { command.query(iid, object) }.ok()
            })
        }

        fn LockServer(&self, lock: BOOL) -> windows_core::Result<()> {
            if lock.as_bool() {
                LOCKS.fetch_add(1, Ordering::SeqCst);
            } else {
                LOCKS.fetch_sub(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    /// Hands out the class factory of [`CLSID`], and of no other class.
    #[no_mangle]
    pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, object: *mut *mut c_void) -> HRESULT {
        catch_unwind(AssertUnwindSafe(|| {
            if object.is_null() {
                return E_POINTER;
            }
            // SAFETY: COM hands a writable out pointer; it was checked for null above.
            unsafe { *object = std::ptr::null_mut() };
            if clsid.is_null() || iid.is_null() {
                return E_INVALIDARG;
            }
            // SAFETY: checked for null above; COM passes a valid GUID.
            if unsafe { *clsid } != CLSID {
                return CLASS_E_CLASSNOTAVAILABLE;
            }
            let factory: IClassFactory = Factory { _count: ObjectCount::new() }.into();
            // SAFETY: `iid` and `object` are valid, as checked above.
            unsafe { factory.query(iid, object) }
        }))
        .unwrap_or(E_UNEXPECTED)
    }

    /// The DLL may go when no object of it is alive and no lock is held.
    #[no_mangle]
    pub extern "system" fn DllCanUnloadNow() -> HRESULT {
        if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 {
            S_OK
        } else {
            S_FALSE
        }
    }
}
