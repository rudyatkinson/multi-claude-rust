//! The Windows 11 tier: a sparse package (`resources/explorer/AppxManifest.xml`) whose COM class
//! (`crates/mc-explorer-command`, shipped as `mc_explorer_command_x64.dll`) is the menu command.
//! It is registered unsigned from the install folder with `Add-AppxPackage -ExternalLocation`,
//! which Windows allows in Developer Mode only (`src/main/explorerMenu/index.ts`).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::powershell::{run_powershell, PowerShellError};

/// Identity of the sparse package in `AppxManifest.xml`.
pub const PACKAGE_NAME: &str = "MultiClaude.ExplorerMenu";
/// COM class of the menu command; the same value is in the manifest and the DLL source.
pub const COMMAND_CLSID: &str = "E00837FA-9B7A-49E7-B33B-0115A2B2C52E";
pub const MANIFEST_FILE: &str = "AppxManifest.xml";

/// Leaves the package alone when it is already registered from this folder with this version.
const ENSURE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'
try {
  $dir = $env:MC_DIR
  $manifest = Join-Path $dir 'AppxManifest.xml'
  $want = ([xml](Get-Content -LiteralPath $manifest -Raw)).Package.Identity.Version
  $current = Get-AppxPackage -Name $env:MC_PKG
  $ok = $current -and "$($current.Version)" -eq $want -and "$($current.Status)" -eq 'Ok' -and $current.InstallLocation.TrimEnd('\') -ieq $dir.TrimEnd('\')
  if (-not $ok) {
    if ($current) { $current | Remove-AppxPackage }
    Add-AppxPackage -Register $manifest -ExternalLocation $dir -ForceUpdateFromAnyVersion -ForceApplicationShutdown
  }
} catch {
  [Console]::Error.WriteLine(('{0}|0x{1:X8}' -f $_.Exception.Message, $_.Exception.HResult)); exit 1
}
"#;

const REMOVE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'
try { Get-AppxPackage -Name $env:MC_PKG | Remove-AppxPackage }
catch { [Console]::Error.WriteLine(('{0}|0x{1:X8}' -f $_.Exception.Message, $_.Exception.HResult)); exit 1 }
"#;

const STATUS_SCRIPT: &str = r#"
$ProgressPreference = 'SilentlyContinue'
$p = Get-AppxPackage -Name $env:MC_PKG
if ($p) { [pscustomobject]@{ location = $p.InstallLocation; status = "$($p.Status)" } | ConvertTo-Json -Compress }
"#;

/// The registered package, as `STATUS_SCRIPT` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PackageInfo {
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

impl PackageInfo {
    /// Windows can use the package (not damaged, not waiting for Developer Mode, ...).
    pub fn is_ok(&self) -> bool {
        self.status.as_deref() == Some("Ok")
    }
}

/// None when nothing is registered, or for output that is not one package.
pub fn parse_status(stdout: &str) -> Option<PackageInfo> {
    let text = stdout.trim();
    if text.is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// The values the scripts read: they never get them through string interpolation.
fn env(dir: &Path) -> [(&'static str, String); 2] {
    [("MC_PKG", PACKAGE_NAME.to_owned()), ("MC_DIR", dir.to_string_lossy().into_owned())]
}

async fn run(script: &str, dir: &Path) -> Result<String, PowerShellError> {
    let env = env(dir);
    let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    run_powershell(script, &pairs).await
}

/// Registers the package from `dir`, unless it already is, from there, with this version.
pub async fn ensure(dir: &Path) -> Result<(), PowerShellError> {
    run(ENSURE_SCRIPT, dir).await.map(drop)
}

/// Removes the package; none being registered is fine.
pub async fn remove() -> Result<(), PowerShellError> {
    run(REMOVE_SCRIPT, Path::new("")).await.map(drop)
}

/// The registered package, if any.
pub async fn status() -> Result<Option<PackageInfo>, PowerShellError> {
    run(STATUS_SCRIPT, Path::new("")).await.map(|out| parse_status(&out))
}

/// ` (0xHRESULT)` when the script reported one.
fn hresult_suffix(e: &PowerShellError) -> String {
    e.hresult.as_ref().map(|h| format!(" (0x{h})")).unwrap_or_default()
}

/// What `ExplorerMenuStatus.detail` says after a failed registration.
pub fn registration_error(e: &PowerShellError) -> String {
    format!("Could not add the entry to the Windows 11 menu{}: {}", hresult_suffix(e), e.message)
}

pub fn removal_error(e: &PowerShellError) -> String {
    format!("Could not remove the entry from the Windows 11 menu{}: {}", hresult_suffix(e), e.message)
}

/// Where `AppxManifest.xml` and the DLL are. A packaged build has them in the resource folder,
/// which Tauri puts next to the executable on Windows (`tauri.conf.json` maps
/// `resources/explorer/` to `explorer/`); a development run reads them from the source tree.
pub fn package_dir(dev: bool, resource_dir: Option<&Path>) -> Option<PathBuf> {
    if dev {
        return Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("explorer"));
    }
    resource_dir.map(|dir| dir.join("explorer"))
}

/// `PathResolver::resource_dir` on Windows, without an `AppHandle`: the executable's folder.
pub fn resource_dir_of_this_build() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

pub fn files_present(dir: Option<&Path>) -> bool {
    dir.is_some_and(|dir| dir.join(MANIFEST_FILE).is_file())
}

#[cfg(windows)]
mod registry {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const DEV_MODE_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock";
    /// Present when the user switched Windows 11 back to the classic context menu.
    const CLASSIC_MENU_KEY: &str = r"Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32";

    /// `AllowDevelopmentWithoutDevLicense` is 1: unsigned packages may be registered.
    pub fn developer_mode() -> bool {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(DEV_MODE_KEY)
            .and_then(|key| key.get_value::<u32, _>("AllowDevelopmentWithoutDevLicense"))
            .is_ok_and(|v| v == 1)
    }

    pub fn classic_menu_forced() -> bool {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(CLASSIC_MENU_KEY).is_ok()
    }
}

#[cfg(windows)]
pub use registry::{classic_menu_forced, developer_mode};

#[cfg(not(windows))]
pub fn developer_mode() -> bool {
    false
}

#[cfg(not(windows))]
pub fn classic_menu_forced() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explorer_menu::MENU_TITLE;
    use regex::Regex;

    fn src_tauri() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn explorer_dir() -> PathBuf {
        src_tauri().join("resources").join("explorer")
    }

    /// Every Rust file of the DLL crate, one after the other.
    fn dll_source() -> String {
        let dir = src_tauri().join("..").join("crates").join("mc-explorer-command").join("src");
        let mut files: Vec<PathBuf> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
        files.sort();
        files.iter().map(|p| read(p)).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn use_one_clsid_in_the_manifest_the_dll_source_and_the_app() {
        let manifest = read(&explorer_dir().join(MANIFEST_FILE));
        let in_manifest: Vec<String> = Regex::new(r#"(?:Clsid|Id)="([0-9A-Fa-f]{8}-[0-9A-Fa-f-]{27})""#)
            .unwrap()
            .captures_iter(&manifest)
            .map(|c| c[1].to_ascii_uppercase())
            .collect();
        assert_eq!(in_manifest.len(), 3, "{in_manifest:?}");
        assert!(in_manifest.iter().all(|id| id == COMMAND_CLSID), "{in_manifest:?}");

        let source = dll_source();
        let c = Regex::new(r"CLSID: GUID = GUID::from_u128\(0x([0-9A-F]{8})_([0-9A-F]{4})_([0-9A-F]{4})_([0-9A-F]{4})_([0-9A-F]{12})\)")
            .unwrap()
            .captures(&source)
            .expect("the DLL source defines CLSID with GUID::from_u128");
        assert_eq!(format!("{}-{}-{}-{}-{}", &c[1], &c[2], &c[3], &c[4], &c[5]), COMMAND_CLSID);
    }

    #[test]
    fn agree_on_the_package_name_the_menu_title_and_the_shipped_files() {
        let manifest = read(&explorer_dir().join(MANIFEST_FILE));
        assert!(manifest.contains(&format!(r#"Name="{PACKAGE_NAME}""#)));
        assert!(dll_source().contains(&format!(r#"w!("{MENU_TITLE}")"#)));
        assert!(read(&src_tauri().join("nsis").join("hooks.nsh")).contains(&format!("Get-AppxPackage -Name {PACKAGE_NAME}")));

        let files: Vec<String> = Regex::new(r#"(?:Path|Logo)="([^"]+)"|<Logo>([^<]+)</Logo>"#)
            .unwrap()
            .captures_iter(&manifest)
            .map(|c| c.get(1).or_else(|| c.get(2)).unwrap().as_str().to_owned())
            .collect();
        assert!(files.contains(&"mc_explorer_command_x64.dll".to_owned()), "{files:?}");
        for file in files {
            assert!(explorer_dir().join(&file).is_file(), "{file}");
        }
    }

    /// Machine type and export names of a PE32+ file (what `scripts/build-explorer-dll.ps1` checks).
    fn inspect_pe(buf: &[u8]) -> (u16, Vec<String>) {
        let u16_at = |at: usize| u16::from_le_bytes([buf[at], buf[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]) as usize;
        let pe = u32_at(0x3c);
        assert_eq!(&buf[pe..pe + 4], b"PE\0\0");
        let machine = u16_at(pe + 4);
        let optional = pe + 24;
        assert_eq!(u16_at(optional), 0x20b, "PE32+");
        let sections: Vec<(usize, usize, usize)> = (0..u16_at(pe + 6) as usize)
            .map(|i| optional + u16_at(pe + 20) as usize + i * 40)
            .map(|at| (u32_at(at + 12), u32_at(at + 8).max(u32_at(at + 16)), u32_at(at + 20)))
            .collect();
        let offset = |rva: usize| sections.iter().find(|(va, size, _)| rva >= *va && rva < va + size).map(|(va, _, raw)| rva - va + raw).unwrap();
        let cstr = |rva: usize| {
            let start = offset(rva);
            let end = start + buf[start..].iter().position(|&b| b == 0).unwrap();
            String::from_utf8_lossy(&buf[start..end]).into_owned()
        };
        let export_dir = offset(u32_at(optional + 112));
        let names = offset(u32_at(export_dir + 32));
        let mut exports: Vec<String> = (0..u32_at(export_dir + 24)).map(|i| cstr(u32_at(names + i * 4))).collect();
        exports.sort();
        (machine, exports)
    }

    #[test]
    fn ship_an_x64_dll_that_exports_just_the_com_entry_points() {
        let dll = std::fs::read(explorer_dir().join("mc_explorer_command_x64.dll")).unwrap();
        assert_eq!(inspect_pe(&dll), (0x8664, vec!["DllCanUnloadNow".to_owned(), "DllGetClassObject".to_owned()]));
    }

    #[test]
    fn reads_what_the_status_script_prints() {
        let info = parse_status("{\"location\":\"C:\\\\Users\\\\me\\\\AppData\\\\Local\\\\Multi Claude\\\\explorer\",\"status\":\"Ok\"}\r\n").unwrap();
        assert_eq!(info.location.as_deref(), Some(r"C:\Users\me\AppData\Local\Multi Claude\explorer"));
        assert!(info.is_ok());
        assert!(!parse_status(r#"{"location":"C:\\x","status":"NeedsRemediation"}"#).unwrap().is_ok());
        assert!(!parse_status(r#"{"location":null}"#).unwrap().is_ok());
        assert_eq!(parse_status(""), None);
        assert_eq!(parse_status(" \r\n"), None);
        assert_eq!(parse_status("[1,2]"), None);
    }

    #[test]
    fn passes_values_to_the_scripts_through_the_environment_only() {
        for script in [ENSURE_SCRIPT, REMOVE_SCRIPT, STATUS_SCRIPT] {
            assert!(script.contains("$env:MC_PKG"));
            assert!(!script.contains(PACKAGE_NAME));
        }
        assert!(ENSURE_SCRIPT.contains("$env:MC_DIR"));
        assert!(ENSURE_SCRIPT.contains("-ExternalLocation $dir -ForceUpdateFromAnyVersion -ForceApplicationShutdown"));
        // a single backslash for PowerShell, as in the template literal of index.ts
        assert!(ENSURE_SCRIPT.contains(r"TrimEnd('\') -ieq $dir.TrimEnd('\')"));
        let dir = Path::new(r"C:\Program Files\Multi Claude\explorer");
        assert_eq!(env(dir), [("MC_PKG", PACKAGE_NAME.to_owned()), ("MC_DIR", dir.to_string_lossy().into_owned())]);
    }

    #[test]
    fn words_failures_with_their_hresult() {
        let e = PowerShellError { message: "Deployment failed.".into(), hresult: Some("80073CFF".into()) };
        assert_eq!(registration_error(&e), "Could not add the entry to the Windows 11 menu (0x80073CFF): Deployment failed.");
        let e = PowerShellError { message: "Timed out".into(), hresult: None };
        assert_eq!(removal_error(&e), "Could not remove the entry from the Windows 11 menu: Timed out");
    }

    #[test]
    fn finds_the_package_files_of_a_build() {
        let dev = package_dir(true, None).unwrap();
        assert_eq!(dev, explorer_dir());
        assert!(files_present(Some(&dev)));
        let install = Path::new(r"C:\Users\me\AppData\Local\Multi Claude");
        assert_eq!(package_dir(false, Some(install)), Some(install.join("explorer")));
        assert_eq!(package_dir(false, None), None);
        assert!(!files_present(None));
        assert!(!files_present(Some(install)));
    }
}
