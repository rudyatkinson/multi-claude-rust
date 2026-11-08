//! The menu command: its title and icon, when it shows, and what clicking it does.

use std::cell::Cell;
use std::ffi::c_void;

use windows::Win32::Foundation::{E_FAIL, E_NOTIMPL, E_POINTER};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx};
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl};
use windows::Win32::System::SystemServices::{SFGAO_FILESYSTEM, SFGAO_FOLDER, SFGAO_STREAM};
use windows::Win32::UI::Shell::{
    IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IFolderView, IPersistFolder2, IShellItem, IShellItemArray, IUnknown_QueryService,
    SHCreateItemFromIDList, SHStrDupW, ECF_DEFAULT, ECS_ENABLED, ECS_HIDDEN, SIGDN_FILESYSPATH,
};
use windows_core::{implement, w, IUnknown, Interface, Ref, BOOL, GUID, PCWSTR, PWSTR};

use crate::launch::{app_exe, launch_app, wide_nul};
use crate::server::{guard, ObjectCount};

/// The text of the menu entry.
const TITLE: PCWSTR = w!("Open in Multi Claude");

#[implement(IExplorerCommand, IObjectWithSite)]
pub(crate) struct Command {
    /// What Explorer set through `IObjectWithSite`: the way to the view hosting the menu.
    site: Cell<Option<IUnknown>>,
    _count: ObjectCount,
}

impl Command {
    pub(crate) fn new() -> Self {
        Command { site: Cell::new(None), _count: ObjectCount::new() }
    }

    /// A reference to the site; the cell keeps its own.
    fn site(&self) -> Option<IUnknown> {
        let site = self.site.take();
        let copy = site.clone();
        self.site.set(site);
        copy
    }
}

/// The folder shown by the view that hosts the menu, for when Explorer passes no selection.
fn folder_from_site(site: Option<&IUnknown>) -> windows_core::Result<IShellItem> {
    let site = site.ok_or(E_FAIL)?;
    // SID_SFolderView is defined as IID_IFolderView in shlguid.h
    // SAFETY: plain COM calls on live interfaces; the PIDL is freed once the item is made.
    unsafe {
        let view: IFolderView = IUnknown_QueryService(site, &IFolderView::IID)?;
        match view.GetFolder::<IShellItem>() {
            Ok(item) => Ok(item),
            Err(_) => {
                let folder: IPersistFolder2 = view.GetFolder()?;
                let pidl = folder.GetCurFolder()?;
                let item = SHCreateItemFromIDList(pidl);
                CoTaskMemFree(Some(pidl as *const c_void));
                item
            }
        }
    }
}

/// Right-clicking a folder passes it as the selection, and so does a click on a folder's
/// background on Windows 11. The site is the fallback for an empty selection.
fn resolve_folder(items: Option<&IShellItemArray>, site: Option<&IUnknown>) -> windows_core::Result<IShellItem> {
    if let Some(items) = items {
        // SAFETY: COM calls on a live interface.
        if let Ok(count) = unsafe { items.GetCount() } {
            if count > 0 {
                return unsafe { items.GetItemAt(0) };
            }
        }
    }
    folder_from_site(site)
}

/// The item's path when it is a real directory; zip files and virtual folders are refused.
fn folder_path(item: &IShellItem) -> windows_core::Result<Vec<u16>> {
    // SAFETY: COM calls on a live interface; the returned string is copied, then freed.
    unsafe {
        let attrs = item.GetAttributes(SFGAO_FILESYSTEM | SFGAO_FOLDER | SFGAO_STREAM)?;
        let has = |flag| (attrs & flag) == flag;
        if !has(SFGAO_FILESYSTEM) || !has(SFGAO_FOLDER) || has(SFGAO_STREAM) {
            return Err(E_FAIL.into());
        }
        let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let wide = path.as_wide().to_vec();
        CoTaskMemFree(Some(path.0 as *const c_void));
        Ok(wide)
    }
}

impl IExplorerCommand_Impl for Command_Impl {
    fn GetTitle(&self, _items: Ref<IShellItemArray>) -> windows_core::Result<PWSTR> {
        // SAFETY: TITLE is a static, NUL-terminated string.
        guard(|| unsafe { SHStrDupW(TITLE) })
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> windows_core::Result<PWSTR> {
        guard(|| {
            let exe = wide_nul(app_exe().ok_or(E_FAIL)?.as_os_str());
            // SAFETY: `exe` is NUL-terminated and outlives the call.
            unsafe { SHStrDupW(PCWSTR(exe.as_ptr())) }
        })
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> windows_core::Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> windows_core::Result<GUID> {
        Ok(GUID::zeroed())
    }

    /// Shown only when the app is there and the folder is a real directory.
    fn GetState(&self, items: Ref<IShellItemArray>, _ok_to_be_slow: BOOL) -> windows_core::Result<u32> {
        guard(|| {
            let shown = app_exe().is_some() && resolve_folder(items.as_ref(), self.site().as_ref()).and_then(|folder| folder_path(&folder)).is_ok();
            Ok(if shown { ECS_ENABLED.0 as u32 } else { ECS_HIDDEN.0 as u32 })
        })
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _bind: Ref<IBindCtx>) -> windows_core::Result<()> {
        guard(|| {
            let folder = resolve_folder(items.as_ref(), self.site().as_ref())?;
            launch_app(&folder_path(&folder)?)
        })
    }

    fn GetFlags(&self) -> windows_core::Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> windows_core::Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

impl IObjectWithSite_Impl for Command_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> windows_core::Result<()> {
        // the old site is released after the cell holds the new one
        drop(self.site.replace(site.cloned()));
        Ok(())
    }

    fn GetSite(&self, iid: *const GUID, site: *mut *mut c_void) -> windows_core::Result<()> {
        guard(|| {
            if site.is_null() {
                return Err(E_POINTER.into());
            }
            // SAFETY: COM hands a writable out pointer; it was checked for null above.
            unsafe { *site = std::ptr::null_mut() };
            match self.site() {
                // SAFETY: `site` is valid; QueryInterface checks `iid`.
                Some(current) => unsafe { current.query(iid, site) }.ok(),
                None => Err(E_FAIL.into()),
            }
        })
    }
}
