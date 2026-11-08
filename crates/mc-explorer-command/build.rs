//! Link settings of the DLL (MSVC only); they apply to the cdylib, not to the test executables.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let link = |arg: &str| println!("cargo:rustc-cdylib-link-arg={arg}");

    // The VC runtime statically, the Universal CRT (part of Windows) dynamically, as Tauri links
    // the app (`build > windows > staticVCRuntime`, the static_vcruntime crate): the DLL must load
    // into dllhost.exe on a machine without the Visual C++ Redistributable.
    for lib in ["libvcruntimed.lib", "vcruntime.lib", "vcruntimed.lib", "libcmtd.lib", "msvcrt.lib", "msvcrtd.lib", "libucrt.lib", "libucrtd.lib"] {
        link(&format!("/NODEFAULTLIB:{lib}"));
    }
    for lib in ["libcmt.lib", "libvcruntime.lib", "ucrt.lib"] {
        link(&format!("/DEFAULTLIB:{lib}"));
    }

    // rustc exports DllGetClassObject and DllCanUnloadNow through a .def file of its own, without
    // the PRIVATE keyword a hand-written one gives COM entry points (keeping them out of the import
    // library, which nothing uses here), and link.exe warns about that (LNK4104).
    link("/IGNORE:4104");
}
