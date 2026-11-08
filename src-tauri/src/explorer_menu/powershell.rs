//! Windows PowerShell for the Appx cmdlets (`src/main/explorerMenu/powershell.ts`). They do not
//! load in PowerShell 7, so this is always `%SystemRoot%\System32\WindowsPowerShell\v1.0`.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

/// How long a script may run; registering a package takes a few seconds.
const TIMEOUT: Duration = Duration::from_secs(60);

/// Put before every script: what it prints reaches us as UTF-8 rather than in the OEM code page
/// (error messages are localized; failing to switch changes nothing else), and no progress
/// records are written, from the first line on.
const PRELUDE: &str = "try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch {}\n$ProgressPreference = 'SilentlyContinue'\n";

/// A failed script. It reports the failure by writing `message|0xHRESULT` to stderr and exiting
/// non-zero; anything else it printed becomes the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerShellError {
    pub message: String,
    /// HRESULT of the failure as 8 upper-case hex digits, when the script reported one.
    pub hresult: Option<String>,
}

impl PowerShellError {
    fn plain(message: impl Into<String>) -> Self {
        PowerShellError { message: message.into(), hresult: None }
    }
}

impl std::fmt::Display for PowerShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PowerShellError {}

/// Standard base64 (RFC 4648, with padding).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let triple = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((triple >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What `-EncodedCommand` takes: the script as UTF-16LE, in base64.
pub fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

/// Windows PowerShell started with `-EncodedCommand` writes progress and error records to stderr
/// as CLIXML (`#< CLIXML` and `<Objs ...>` blocks); only what the script wrote itself is kept.
fn without_clixml(stderr: &str) -> String {
    static OBJECTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<Objs\b.*?</Objs>").expect("valid regex"));
    if !stderr.contains("#< CLIXML") {
        return stderr.to_owned();
    }
    let rest = OBJECTS.replace_all(stderr, "");
    rest.split_inclusive('\n').filter(|line| line.trim() != "#< CLIXML").collect()
}

/// Splits `message|0xHRESULT` off what a failed script printed; `fallback` stands in for silence.
pub fn parse_failure(stderr: &str, fallback: &str) -> PowerShellError {
    static FAILURE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^(.*)\|0x([0-9A-Fa-f]{8})\s*$").expect("valid regex"));
    let stderr = without_clixml(stderr);
    let text = stderr.trim();
    let text = if text.is_empty() { fallback } else { text };
    match FAILURE.captures(text) {
        Some(c) => PowerShellError { message: c[1].trim().to_owned(), hresult: Some(c[2].to_ascii_uppercase()) },
        None => PowerShellError::plain(text),
    }
}

fn powershell_exe() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").filter(|v| !v.is_empty()).unwrap_or_else(|| r"C:\Windows".into());
    std::path::Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

fn decode(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
}

/// Runs `script` without a window and returns its stdout. Values reach the script only through
/// `env`, never by string interpolation.
pub async fn run_powershell(script: &str, env: &[(&str, &str)]) -> Result<String, PowerShellError> {
    let encoded = encode_command(&format!("{PRELUDE}{script}"));
    let mut command = tokio::process::Command::new(powershell_exe());
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded])
        // a module path inherited from PowerShell 7 makes Windows PowerShell load incompatible
        // modules (the name is matched without regard to case on Windows)
        .env_remove("PSModulePath")
        .envs(env.iter().copied())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match tokio::time::timeout(TIMEOUT, command.output()).await {
        Ok(Ok(out)) if out.status.success() => Ok(decode(&out.stdout)),
        Ok(Ok(out)) => Err(parse_failure(&decode(&out.stderr), &format!("Windows PowerShell failed ({})", out.status))),
        Ok(Err(e)) => Err(PowerShellError::plain(format!("Could not start Windows PowerShell: {e}"))),
        Err(_) => Err(PowerShellError::plain("Windows PowerShell did not finish in time")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_base64_like_rfc_4648() {
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(input.as_bytes()), expected, "{input}");
        }
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }

    #[test]
    fn encodes_a_script_as_utf16le_base64() {
        // Buffer.from('dir', 'utf16le').toString('base64')
        assert_eq!(encode_command("dir"), "ZABpAHIA");
        // a character outside the BMP is a surrogate pair
        assert_eq!(encode_command("ı😀"), base64(&[0x31, 0x01, 0x3d, 0xd8, 0x00, 0xde]));
    }

    #[test]
    fn splits_the_hresult_off_a_failure() {
        assert_eq!(
            parse_failure("Deployment failed with HRESULT: 0x80073CF0.|0x80073cf0\r\n", "x"),
            PowerShellError { message: "Deployment failed with HRESULT: 0x80073CF0.".into(), hresult: Some("80073CF0".into()) }
        );
        assert_eq!(
            parse_failure("first line\r\nsecond line|0x80070005", "x"),
            PowerShellError { message: "first line\r\nsecond line".into(), hresult: Some("80070005".into()) }
        );
        assert_eq!(parse_failure("  just text  ", "x"), PowerShellError::plain("just text"));
        assert_eq!(parse_failure("text|0x123", "x"), PowerShellError::plain("text|0x123"));
        assert_eq!(parse_failure(" \r\n", "exit code 1"), PowerShellError::plain("exit code 1"));
    }

    #[test]
    fn ignores_the_clixml_records_powershell_adds() {
        let progress = "<Objs Version=\"1.1.0.1\" xmlns=\"http://schemas.microsoft.com/powershell/2004/04\"><Obj S=\"progress\" RefId=\"0\"><MS><PR N=\"Record\"><AV>Preparing modules for first use.</AV></PR></MS></Obj></Objs>";
        let stderr = format!("#< CLIXML\r\nErişim engellendi\r\nline two|0x80070005\r\n{progress}");
        assert_eq!(parse_failure(&stderr, "x"), PowerShellError { message: "Erişim engellendi\r\nline two".into(), hresult: Some("80070005".into()) });
        assert_eq!(parse_failure(&format!("#< CLIXML\r\n{progress}"), "exit code 1"), PowerShellError::plain("exit code 1"));
    }

    /// Spawns Windows PowerShell, which every Windows has; changes nothing.
    #[cfg(windows)]
    #[tokio::test]
    async fn runs_a_script_with_its_environment_and_reads_failures() {
        let out = run_powershell("[Console]::Out.Write(\"$env:MC_TEST|ıİşğ|$([bool]$env:PSModulePath)\")", &[("MC_TEST", "C:\\a b\\ç")]).await.unwrap();
        // Windows PowerShell sets its own PSModulePath when none is inherited
        assert!(out.starts_with("C:\\a b\\ç|ıİşğ|"), "{out}");

        let json = run_powershell("[pscustomobject]@{ location = $env:MC_TEST; status = 'Ok' } | ConvertTo-Json -Compress", &[("MC_TEST", "D:\\ü")]).await.unwrap();
        assert_eq!(json.trim(), r#"{"location":"D:\\ü","status":"Ok"}"#);

        let err = run_powershell("[Console]::Error.WriteLine('Erişim engellendi|0x80070005'); exit 1", &[]).await.unwrap_err();
        assert_eq!(err, PowerShellError { message: "Erişim engellendi".into(), hresult: Some("80070005".into()) });

        let err = run_powershell("exit 3", &[]).await.unwrap_err();
        assert!(err.message.contains('3') && err.hresult.is_none(), "{err:?}");
    }
}
