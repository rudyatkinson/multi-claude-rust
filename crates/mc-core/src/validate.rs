//! Argument validation for commands: the checks `src/main/ipc.ts` of the Electron version made on
//! what the window sent, with the same error wording (the renderer shows the message as it is).
//!
//! Every length is a JavaScript `length`, counted in UTF-16 units (see [`crate::jsstr`]).
//!
//! Plain values arrive typed (`&str`, `&[String]`, `f64`); the compound arguments that the
//! TypeScript read field by field from an unchecked object (`sessionInput`, `placeOf`, the refs,
//! the settings patch, ...) take a [`serde_json::Value`], so that a missing or mistyped field gets
//! the same message it got there instead of a serde error.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::jsstr::{cut16, longer16};
use crate::types::{
    AgentRef, CreateSessionInput, InstructionRef, InstructionTarget, MemoryCreateInput, MemoryRef, Place, ProfilePatch,
    ResumeMode, SettingsPatch, SkillRef, StartRunInput, TabStrip, TransferMode, FONT_SIZE, INSTRUCTION_LIMITS, MEMORY_LIMITS,
    WORKFLOW_LIMITS,
};

/// The default limit of `str`.
pub const STR_MAX: usize = 4096;
/// The limit of an id: a session, a profile, a run, a workflow, a conversation.
pub const ID_MAX: usize = 64;
/// `sessions:closeMany` takes at most this many ids.
pub const IDS_MAX: usize = 500;
/// `mcp:copy` takes at most this many names.
pub const MCP_NAMES_MAX: usize = 200;
/// `clipboard:writeText` takes at most this many UTF-16 units.
pub const CLIPBOARD_MAX: usize = 10_000_000;
pub const MEMORY_QUERY_MAX: usize = 200;
pub const TAB_NAME_MAX: usize = 80;
pub const ATTENTION_TITLE_MAX: usize = 200;
/// The limits of a pty size, both ways.
pub const PTY_MIN: u16 = 2;
pub const PTY_MAX: u16 = 1000;

static ROOT_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[pj]-[0-9a-f]{12}$").expect("valid regex"));
static MEMORY_FOLDER_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^m-[0-9a-f]{12}$").expect("valid regex"));
static SCOPE_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[af]-[0-9a-f]{12}$").expect("valid regex"));
static HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}$").expect("valid regex"));
static MCP_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,64}$").expect("valid regex"));

fn invalid(what: &str) -> AppError {
    AppError::invalid(format!("Invalid {what}"))
}

// ---------- texts ----------

/// `str(value, what, max)`: a text of 1 to `max` UTF-16 units, else `Invalid <what>`.
pub fn str<'a>(value: &'a str, what: &str, max: usize) -> AppResult<&'a str> {
    if value.is_empty() || longer16(value, max) {
        return Err(invalid(what));
    }
    Ok(value)
}

/// An id as the window names it: `str(value, what, 64)`.
pub fn id<'a>(value: &'a str, what: &str) -> AppResult<&'a str> {
    str(value, what, ID_MAX)
}

/// `text(value, what, max)`: a text that may be empty, of at most `max` UTF-16 units.
pub fn text<'a>(value: &'a str, what: &str, max: usize) -> AppResult<&'a str> {
    if longer16(value, max) {
        return Err(invalid(what));
    }
    Ok(value)
}

/// `file === undefined || file === null ? undefined : str(file, what, max)`.
pub fn optional_str<'a>(value: Option<&'a str>, what: &str, max: usize) -> AppResult<Option<&'a str>> {
    value.map(|v| str(v, what, max)).transpose()
}

/// `clipboard:writeText`: `str(text, 'text', 10_000_000)`. An empty text is refused as well.
pub fn clipboard_text(value: &str) -> AppResult<&str> {
    str(value, "text", CLIPBOARD_MAX)
}

/// `memory:search`: `text(query, 'query', 200)`.
pub fn memory_query(value: &str) -> AppResult<&str> {
    text(value, "query", MEMORY_QUERY_MAX)
}

/// The whitespace `String.prototype.trim` removes: WhiteSpace and LineTerminator of ECMAScript.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{D}'
            | ' '
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// `s.trim()` in JavaScript. `str::trim` differs at U+0085 and U+FEFF.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `tabName`: a tab name as typed, with control characters as spaces, trimmed and cut to 80
/// UTF-16 units; None when nothing is left, which gives the tab its automatic title back.
pub fn tab_name(value: &str) -> Option<String> {
    // control characters would garble the tab bar and the saved file
    let clean: String = value.chars().map(|c| if (c as u32) < 32 || c as u32 == 127 { ' ' } else { c }).collect();
    let name = cut16(js_trim(&clean), TAB_NAME_MAX);
    (!name.is_empty()).then(|| name.to_owned())
}

/// `session:attention`: the title, cut to 200 UTF-16 units.
pub fn attention_title(title: &str) -> &str {
    cut16(title, ATTENTION_TITLE_MAX)
}

// ---------- ids handed out by the backend ----------

/// `skillRoot`: the id of a skills (or agents) folder, `p-` or `j-` and 12 hex digits.
pub fn skill_root(value: &str) -> AppResult<&str> {
    let id = str(value, "folder", 32)?;
    if !ROOT_ID.is_match(id) {
        return Err(invalid("folder"));
    }
    Ok(id)
}

/// `memoryFolder`: the id of a memory folder, `m-` and 12 hex digits.
pub fn memory_folder(value: &str) -> AppResult<&str> {
    let id = str(value, "folder", 32)?;
    if !MEMORY_FOLDER_ID.is_match(id) {
        return Err(invalid("folder"));
    }
    Ok(id)
}

/// `instructionScope`: the id of an account's (`a-`) or a project's (`f-`) instruction files.
pub fn instruction_scope(value: &str) -> AppResult<&str> {
    let id = str(value, "folder", 32)?;
    if !SCOPE_ID.is_match(id) {
        return Err(invalid("folder"));
    }
    Ok(id)
}

/// `baseOf`: the state a save started from, the hash of a file, or None to overwrite.
pub fn base_hash(value: Option<&str>) -> AppResult<Option<&str>> {
    match value {
        None => Ok(None),
        Some(hash) if HASH.is_match(hash) => Ok(Some(hash)),
        Some(_) => Err(invalid("state")),
    }
}

// ---------- lists ----------

/// `sessions:closeMany`: at most 500 session ids.
pub fn ids_many(ids: &[String]) -> AppResult<Vec<&str>> {
    if ids.len() > IDS_MAX {
        return Err(invalid("sessions"));
    }
    ids.iter().map(|value| id(value, "session")).collect()
}

/// `mcp:copy`: at most 200 server names of at most 64 units. Like the Electron version this does
/// not apply [`is_mcp_server_name`]: a name Claude Code would not accept is not among the main
/// account's servers, so the copy reports it as such for that one name.
pub fn mcp_names(names: &[String]) -> AppResult<Vec<&str>> {
    if names.len() > MCP_NAMES_MAX {
        return Err(invalid("servers"));
    }
    names.iter().map(|name| str(name, "server", ID_MAX)).collect()
}

/// The names Claude Code accepts for an MCP server (`VALID_NAME` of `mcpServers.ts`).
pub fn is_mcp_server_name(name: &str) -> bool {
    MCP_NAME.is_match(name)
}

// ---------- paths ----------

/// `path.isAbsolute` of Node for this platform. On Windows a path rooted without a drive
/// (`\foo`) counts as absolute, as Node counts it and `Path::is_absolute` does not.
fn is_absolute_js(p: &str) -> bool {
    let b = p.as_bytes();
    if cfg!(windows) {
        let sep = |c: u8| c == b'/' || c == b'\\';
        b.first().is_some_and(|&c| sep(c)) || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2]))
    } else {
        b.first() == Some(&b'/')
    }
}

/// `folder(value)`: a folder as the window named it, once it is known to exist.
fn folder_str(value: &str) -> AppResult<&str> {
    let cwd = str(value, "folder", STR_MAX)?;
    if !Path::new(cwd).is_dir() {
        return Err(AppError::refused(format!("Folder not found: {cwd}")));
    }
    Ok(cwd)
}

/// `folder(value)`: `Invalid folder` for a bad text, `Folder not found: <path>` when it is not a
/// folder on disk. This touches the file system.
pub fn folder_path(value: &str) -> AppResult<PathBuf> {
    folder_str(value).map(PathBuf::from)
}

/// `fs:folderFor`'s check of a dropped path: `Invalid path` for a bad text, None for a path that
/// is not absolute (the Electron handler answered null), else the path.
pub fn path(value: &str) -> AppResult<Option<PathBuf>> {
    let target = str(value, "path", STR_MAX)?;
    Ok(is_absolute_js(target).then(|| PathBuf::from(target)))
}

// ---------- numbers ----------

/// `pty:resize`: whole numbers from 2 to 1000 both ways, else nothing happens.
pub fn pty_size(cols: f64, rows: f64) -> Option<(u16, u16)> {
    let fits = |n: f64| n.fract() == 0.0 && (f64::from(PTY_MIN)..=f64::from(PTY_MAX)).contains(&n);
    (fits(cols) && fits(rows)).then_some((cols as u16, rows as u16))
}

/// `Math.round`: halves go up, also below zero.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// The font size of a settings patch: rounded and held within [`FONT_SIZE`]; None for a value
/// that is not a finite number.
pub fn font_size(value: f64) -> Option<u32> {
    if !value.is_finite() {
        return None;
    }
    Some(js_round(value).clamp(f64::from(FONT_SIZE.min), f64::from(FONT_SIZE.max)) as u32)
}

/// `workflows:output`: the index of a step, a whole number below the step limit.
pub fn step_index(index: f64) -> AppResult<usize> {
    if index.fract() != 0.0 || index < 0.0 || index >= WORKFLOW_LIMITS.steps as f64 {
        return Err(invalid("step"));
    }
    Ok(index as usize)
}

/// `=== true`: only a real `true` counts (`force`, `continueLast`, `apply`, `enabled`).
pub fn flag(value: &Value) -> bool {
    value == &Value::Bool(true)
}

// ---------- links ----------

/// The code points `new URL` percent-encodes in a part besides C0 controls and non-ASCII.
const PATH_SET: &[char] = &[' ', '"', '#', '<', '>', '?', '`', '{', '}'];
const QUERY_SET: &[char] = &[' ', '"', '#', '<', '>', '\''];
const FRAGMENT_SET: &[char] = &[' ', '"', '<', '>', '`'];

fn percent_encode(part: &str, set: &[char]) -> String {
    let mut out = String::with_capacity(part.len());
    for c in part.chars() {
        if (c as u32) < 0x20 || (c as u32) > 0x7e || set.contains(&c) {
            let mut buf = [0u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Host and port of an authority, the port left out when it is the scheme's default.
fn host_port<'a>(hostport: &'a str, scheme: &str) -> AppResult<(&'a str, Option<u16>)> {
    // an IPv6 address is in brackets and has colons of its own
    let split = match hostport.rfind(':') {
        Some(at) if !hostport[at..].contains(']') => Some(at),
        _ => None,
    };
    let (host, port) = match split {
        Some(at) => (&hostport[..at], Some(&hostport[at + 1..])),
        None => (hostport, None),
    };
    const FORBIDDEN: &[char] = &[' ', '#', '%', '/', ':', '<', '>', '?', '@', '\\', '^', '|'];
    let bracketed = host.starts_with('[') && host.ends_with(']') && host.len() > 2;
    let bad_host = host.is_empty()
        || host.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f)
        || (!bracketed && (host.contains(['[', ']']) || host.contains(FORBIDDEN)));
    if bad_host {
        return Err(invalid("URL"));
    }
    let port = match port {
        None | Some("") => None,
        Some(digits) => {
            if !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid("URL"));
            }
            let n: u16 = digits.parse().map_err(|_| invalid("URL"))?;
            let default = if scheme == "https" { 443 } else { 80 };
            (n != default).then_some(n)
        }
    };
    Ok((host, port))
}

/// `shell:openExternal`: a link that may be handed to the system, http or https only.
///
/// The Electron version took the text apart with `new URL` and opened `parsed.toString()`. This
/// is close to it: `Invalid URL` for what does not parse, `Only http(s) links can be opened` for
/// any other scheme, and the link with a lower-case scheme and host, `//` after the scheme, no
/// default port, `/` for an empty path, backslashes in the path as slashes, and the characters
/// the URL standard encodes percent-encoded. It does not resolve `.` and `..` segments, convert
/// international domain names or rewrite IP addresses.
pub fn url_http(value: &str) -> AppResult<String> {
    let raw = str(value, "URL", STR_MAX)?;
    // the URL parser drops C0 controls and spaces around the text, and tabs and newlines in it
    let s: String = raw.trim_matches(|c: char| c <= ' ').chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    let colon = s.find(':').ok_or_else(|| invalid("URL"))?;
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    let scheme_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        // no scheme: a relative link, which has nothing to be relative to
        return Err(invalid("URL"));
    }
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(AppError::invalid("Only http(s) links can be opened"));
    }

    let rest = s[colon + 1..].trim_start_matches(['/', '\\']);
    let end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
        None => (None, authority),
    };
    let (host, port) = host_port(hostport, &scheme)?;

    let (before_fragment, fragment) = match tail.find('#') {
        Some(at) => (&tail[..at], Some(&tail[at + 1..])),
        None => (tail, None),
    };
    let (path, query) = match before_fragment.find('?') {
        Some(at) => (&before_fragment[..at], Some(&before_fragment[at + 1..])),
        None => (before_fragment, None),
    };
    let path = path.replace('\\', "/");

    let mut out = format!("{scheme}://");
    if let Some(userinfo) = userinfo.filter(|u| !u.is_empty()) {
        out.push_str(userinfo);
        out.push('@');
    }
    out.push_str(&host.to_ascii_lowercase());
    if let Some(port) = port {
        out.push_str(&format!(":{port}"));
    }
    if path.is_empty() {
        out.push('/');
    }
    out.push_str(&percent_encode(&path, PATH_SET));
    if let Some(query) = query {
        out.push('?');
        out.push_str(&percent_encode(query, QUERY_SET));
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(&percent_encode(fragment, FRAGMENT_SET));
    }
    Ok(out)
}

// ---------- compound arguments ----------

/// A field of an object; nothing for anything that is not an object, as `(value ?? {}).key` is.
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object()?.get(key)
}

/// The text of a field, or `Invalid <what>` when it is missing or not a text.
fn text_of<'a>(value: Option<&'a Value>, what: &str) -> AppResult<&'a str> {
    value.and_then(Value::as_str).ok_or_else(|| invalid(what))
}

/// An id of a place that passes: a text of 1 to 64 units.
fn place_id(value: Option<&Value>) -> Option<String> {
    let id = value?.as_str()?;
    (!id.is_empty() && !longer16(id, ID_MAX)).then(|| id.to_owned())
}

/// `placeOf`: where a new tab should appear, as the window asked. Only short ids pass; anything
/// that is not an object is no place. (A list is an object to JavaScript: an empty place.)
pub fn place(value: &Value) -> Option<Place> {
    match value {
        Value::Object(_) => Some(Place { group_id: place_id(field(value, "groupId")), after_id: place_id(field(value, "afterId")) }),
        Value::Array(_) => Some(Place::default()),
        _ => None,
    }
}

/// `sessionInput`: what `sessions:create` and `sessions:openExternal` take. The folder is checked
/// first and must exist; a resume mode other than `last` or `pick` is dropped.
pub fn session_input(value: &Value) -> AppResult<CreateSessionInput> {
    let cwd = folder_str(text_of(field(value, "cwd"), "folder")?)?;
    let resume = match field(value, "resume").and_then(Value::as_str) {
        Some("last") => Some(ResumeMode::Last),
        Some("pick") => Some(ResumeMode::Pick),
        _ => None,
    };
    let profile_id = id(text_of(field(value, "profileId"), "profile")?, "profile")?;
    let place = field(value, "place").and_then(place);
    Ok(CreateSessionInput { profile_id: profile_id.to_owned(), cwd: cwd.to_owned(), resume, place })
}

/// `skillRef`: a skill as the window names it. Whether its folder exists is for the skill service to say.
pub fn skill_ref(value: &Value) -> AppResult<SkillRef> {
    let root_id = skill_root(text_of(field(value, "rootId"), "folder")?)?;
    let dir_name = str(text_of(field(value, "dirName"), "skill")?, "skill", 255)?;
    Ok(SkillRef { root_id: root_id.to_owned(), dir_name: dir_name.to_owned() })
}

/// `agentRef`: an agent as the window names it.
pub fn agent_ref(value: &Value) -> AppResult<AgentRef> {
    let root_id = skill_root(text_of(field(value, "rootId"), "folder")?)?;
    let file = str(text_of(field(value, "file"), "agent")?, "agent", 400)?;
    Ok(AgentRef { root_id: root_id.to_owned(), file: file.to_owned() })
}

/// `memoryRef`: a memory as the window names it.
pub fn memory_ref(value: &Value) -> AppResult<MemoryRef> {
    let folder_id = memory_folder(text_of(field(value, "folderId"), "folder")?)?;
    let file = str(text_of(field(value, "file"), "memory")?, "memory", 255)?;
    Ok(MemoryRef { folder_id: folder_id.to_owned(), file: file.to_owned() })
}

/// `instructionRef`: an instruction file as the window names it.
pub fn instruction_ref(value: &Value) -> AppResult<InstructionRef> {
    let scope_id = instruction_scope(text_of(field(value, "scopeId"), "folder")?)?;
    let file = str(text_of(field(value, "file"), "file")?, "file", 400)?;
    Ok(InstructionRef { scope_id: scope_id.to_owned(), file: file.to_owned() })
}

/// `instructions:create`'s target: a rule when `rule` is a text, else one of the fixed files.
pub fn instruction_target(value: &Value) -> AppResult<InstructionTarget> {
    if let Some(rule) = field(value, "rule").and_then(Value::as_str) {
        return Ok(InstructionTarget::Rule(str(rule, "name", INSTRUCTION_LIMITS.rule_name)?.to_owned()));
    }
    Ok(InstructionTarget::File(str(text_of(field(value, "file"), "file")?, "file", 64)?.to_owned()))
}

/// `memory:create`'s input: a name, a title and a description that may be empty, and a type.
pub fn memory_create_input(value: &Value) -> AppResult<MemoryCreateInput> {
    let name = str(text_of(field(value, "name"), "name")?, "name", MEMORY_LIMITS.name)?;
    let title = text(text_of(field(value, "title"), "title")?, "title", MEMORY_LIMITS.title)?;
    let description = text(text_of(field(value, "description"), "description")?, "description", MEMORY_LIMITS.description)?;
    let kind = str(text_of(field(value, "type"), "type")?, "type", 20)?;
    Ok(MemoryCreateInput { name: name.to_owned(), title: title.to_owned(), description: description.to_owned(), r#type: kind.to_owned() })
}

/// `workflows:run`'s input: the task first (it may be empty), then the workflow and the folder,
/// which must exist.
pub fn start_run_input(value: &Value) -> AppResult<StartRunInput> {
    let task = match field(value, "task").and_then(Value::as_str) {
        Some(task) if !longer16(task, WORKFLOW_LIMITS.task) => task,
        _ => return Err(invalid("task")),
    };
    let workflow_id = id(text_of(field(value, "workflowId"), "workflow")?, "workflow")?;
    let cwd = folder_str(text_of(field(value, "cwd"), "folder")?)?;
    Ok(StartRunInput { workflow_id: workflow_id.to_owned(), cwd: cwd.to_owned(), task: task.to_owned() })
}

/// `skills:transfer` and `agents:transfer`: `copy` or `move`.
pub fn transfer_mode(value: &str) -> AppResult<TransferMode> {
    match value {
        "copy" => Ok(TransferMode::Copy),
        "move" => Ok(TransferMode::Move),
        _ => Err(invalid("mode")),
    }
}

/// `profiles:update`'s patch: each field only when it has the right type.
pub fn profile_patch(value: &Value) -> ProfilePatch {
    let text = |key: &str| field(value, key).and_then(Value::as_str).map(str::to_owned);
    ProfilePatch { name: text("name"), color: text("color"), share_memory: field(value, "shareMemory").and_then(Value::as_bool) }
}

/// `settings:update`'s patch. Only these keys pass: `claudePath` (trimmed; anything but a text
/// with something in it clears it), `fontSize` (rounded and held within the limits),
/// `restoreTabs`, `notifications`, `limitAlerts`, `autoSwitch`, `tray` and `openAtLogin`
/// (booleans), `tabStrip` (`top` or `side`). The rest of the settings are the backend's to set.
pub fn settings_patch(value: &Value) -> SettingsPatch {
    let Some(p) = value.as_object() else {
        return SettingsPatch::default();
    };
    let flag = |key: &str| p.get(key).and_then(Value::as_bool);
    SettingsPatch {
        claude_path: p.get("claudePath").map(|v| v.as_str().map(js_trim).filter(|s| !s.is_empty()).map(str::to_owned)),
        font_size: p.get("fontSize").and_then(Value::as_f64).and_then(font_size),
        restore_tabs: flag("restoreTabs"),
        notifications: flag("notifications"),
        limit_alerts: flag("limitAlerts"),
        auto_switch: flag("autoSwitch"),
        tray: flag("tray"),
        open_at_login: flag("openAtLogin"),
        tab_strip: p.get("tabStrip").and_then(Value::as_str).and_then(|s| match s {
            "top" => Some(TabStrip::Top),
            "side" => Some(TabStrip::Side),
            _ => None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message<T: std::fmt::Debug>(result: AppResult<T>) -> String {
        result.expect_err("should be refused").to_string()
    }

    #[test]
    fn str_counts_utf16_units() {
        assert_eq!(str("abc", "name", 3).unwrap(), "abc");
        assert_eq!(message(str("", "name", 3)), "Invalid name");
        assert_eq!(message(str("abcd", "name", 3)), "Invalid name");
        // two emoji are four units
        assert_eq!(message(str("😀😀", "name", 3)), "Invalid name");
        assert!(str("😀😀", "name", 4).is_ok());
        assert!(str(&"x".repeat(STR_MAX), "folder", STR_MAX).is_ok());
        assert!(str(&"x".repeat(STR_MAX + 1), "folder", STR_MAX).is_err());
    }

    #[test]
    fn id_is_at_most_64() {
        assert!(id(&"a".repeat(64), "session").is_ok());
        assert_eq!(message(id(&"a".repeat(65), "session")), "Invalid session");
        assert_eq!(message(id("", "profile")), "Invalid profile");
    }

    #[test]
    fn text_may_be_empty() {
        assert_eq!(text("", "description", 10).unwrap(), "");
        assert_eq!(message(text("abcdefghijk", "description", 10)), "Invalid description");
    }

    #[test]
    fn optional_str_passes_none() {
        assert_eq!(optional_str(None, "file", 400).unwrap(), None);
        assert_eq!(optional_str(Some("a.md"), "file", 400).unwrap(), Some("a.md"));
        assert_eq!(message(optional_str(Some(""), "file", 400)), "Invalid file");
    }

    #[test]
    fn clipboard_text_needs_something() {
        assert!(clipboard_text("x").is_ok());
        assert_eq!(message(clipboard_text("")), "Invalid text");
        assert!(clipboard_text(&"x".repeat(CLIPBOARD_MAX)).is_ok());
        assert!(clipboard_text(&"x".repeat(CLIPBOARD_MAX + 1)).is_err());
    }

    #[test]
    fn memory_query_limit() {
        assert_eq!(memory_query("").unwrap(), "");
        assert!(memory_query(&"q".repeat(200)).is_ok());
        assert_eq!(message(memory_query(&"q".repeat(201))), "Invalid query");
    }

    #[test]
    fn tab_names() {
        assert_eq!(tab_name("  Build  ").as_deref(), Some("Build"));
        assert_eq!(tab_name("a\tb\u{1b}[31mc\u{7f}").as_deref(), Some("a b [31mc"));
        assert_eq!(tab_name("   \n\t  "), None);
        assert_eq!(tab_name(""), None);
        // JavaScript trims U+FEFF and leaves U+0085, unlike str::trim
        assert_eq!(tab_name("\u{feff}x\u{feff}").as_deref(), Some("x"));
        assert_eq!(tab_name("x\u{85}").as_deref(), Some("x\u{85}"));
        let long = "n".repeat(100);
        assert_eq!(tab_name(&long).unwrap().len(), 80);
        // the cut is in UTF-16 units and never splits a pair
        let emoji = "😀".repeat(41);
        assert_eq!(tab_name(&emoji).unwrap(), "😀".repeat(40));
    }

    #[test]
    fn attention_title_is_cut() {
        assert_eq!(attention_title(&"t".repeat(300)).len(), 200);
        assert_eq!(attention_title("short"), "short");
    }

    #[test]
    fn root_ids() {
        assert_eq!(skill_root("p-0123456789ab").unwrap(), "p-0123456789ab");
        assert!(skill_root("j-0123456789ab").is_ok());
        assert_eq!(message(skill_root("x-0123456789ab")), "Invalid folder");
        assert_eq!(message(skill_root("p-0123456789AB")), "Invalid folder");
        assert_eq!(message(skill_root("p-0123456789ab\n")), "Invalid folder");
        assert_eq!(message(skill_root("")), "Invalid folder");
        assert!(memory_folder("m-0123456789ab").is_ok());
        assert_eq!(message(memory_folder("p-0123456789ab")), "Invalid folder");
        assert!(instruction_scope("a-0123456789ab").is_ok());
        assert!(instruction_scope("f-0123456789ab").is_ok());
        assert_eq!(message(instruction_scope("m-0123456789ab")), "Invalid folder");
    }

    #[test]
    fn base_hashes() {
        let hash = "0123456789abcdef".repeat(4);
        assert_eq!(base_hash(None).unwrap(), None);
        assert_eq!(base_hash(Some(&hash)).unwrap(), Some(hash.as_str()));
        assert_eq!(message(base_hash(Some("abc"))), "Invalid state");
        assert_eq!(message(base_hash(Some(&hash.to_uppercase()))), "Invalid state");
    }

    #[test]
    fn many_ids() {
        let ids: Vec<String> = (0..500).map(|i| format!("s{i}")).collect();
        assert_eq!(ids_many(&ids).unwrap().len(), 500);
        let more: Vec<String> = (0..501).map(|i| format!("s{i}")).collect();
        assert_eq!(message(ids_many(&more)), "Invalid sessions");
        assert_eq!(message(ids_many(&["ok".into(), String::new()])), "Invalid session");
    }

    #[test]
    fn mcp_server_names() {
        assert_eq!(mcp_names(&["github".into(), "my server".into()]).unwrap(), vec!["github", "my server"]);
        let many: Vec<String> = (0..201).map(|i| format!("s{i}")).collect();
        assert_eq!(message(mcp_names(&many)), "Invalid servers");
        assert_eq!(message(mcp_names(&["x".repeat(65)])), "Invalid server");
        assert!(is_mcp_server_name("git-hub_2"));
        assert!(!is_mcp_server_name("my server"));
        assert!(!is_mcp_server_name(""));
        assert!(!is_mcp_server_name(&"a".repeat(65)));
    }

    #[test]
    fn folders_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let there = dir.path().to_str().unwrap();
        assert_eq!(folder_path(there).unwrap(), dir.path());
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let file = file.to_str().unwrap();
        assert_eq!(message(folder_path(file)), format!("Folder not found: {file}"));
        assert_eq!(message(folder_path("")), "Invalid folder");
    }

    #[test]
    fn dropped_paths() {
        assert_eq!(message(path("")), "Invalid path");
        assert_eq!(path("relative/thing").unwrap(), None);
        if cfg!(windows) {
            assert_eq!(path(r"C:\Users").unwrap(), Some(PathBuf::from(r"C:\Users")));
            assert!(path("C:/Users").unwrap().is_some());
            assert!(path(r"\\server\share").unwrap().is_some());
            assert!(path(r"\rooted").unwrap().is_some());
            assert_eq!(path("C:relative").unwrap(), None);
        } else {
            assert!(path("/home").unwrap().is_some());
            assert_eq!(path(r"C:\Users").unwrap(), None);
        }
    }

    #[test]
    fn pty_sizes() {
        assert_eq!(pty_size(80.0, 24.0), Some((80, 24)));
        assert_eq!(pty_size(2.0, 1000.0), Some((2, 1000)));
        assert_eq!(pty_size(1.0, 24.0), None);
        assert_eq!(pty_size(80.0, 1001.0), None);
        assert_eq!(pty_size(80.5, 24.0), None);
        assert_eq!(pty_size(f64::NAN, 24.0), None);
        assert_eq!(pty_size(f64::INFINITY, 24.0), None);
    }

    #[test]
    fn font_sizes() {
        assert_eq!(font_size(13.0), Some(13));
        assert_eq!(font_size(13.5), Some(14));
        assert_eq!(font_size(13.49), Some(13));
        assert_eq!(font_size(2.0), Some(8));
        assert_eq!(font_size(-100.0), Some(8));
        assert_eq!(font_size(99.0), Some(28));
        assert_eq!(font_size(f64::NAN), None);
        assert_eq!(font_size(f64::INFINITY), None);
    }

    #[test]
    fn step_indexes() {
        assert_eq!(step_index(0.0).unwrap(), 0);
        assert_eq!(step_index(11.0).unwrap(), 11);
        assert_eq!(message(step_index(12.0)), "Invalid step");
        assert_eq!(message(step_index(-1.0)), "Invalid step");
        assert_eq!(message(step_index(1.5)), "Invalid step");
        assert_eq!(message(step_index(f64::NAN)), "Invalid step");
    }

    #[test]
    fn flags() {
        assert!(flag(&json!(true)));
        assert!(!flag(&json!(false)));
        assert!(!flag(&json!("true")));
        assert!(!flag(&json!(1)));
        assert!(!flag(&Value::Null));
    }

    #[test]
    fn links() {
        assert_eq!(url_http("https://example.com").unwrap(), "https://example.com/");
        assert_eq!(url_http("HTTPS://Example.COM/Path?q=a b#frag ment").unwrap(), "https://example.com/Path?q=a%20b#frag%20ment");
        assert_eq!(url_http("http://example.com:80/x").unwrap(), "http://example.com/x");
        assert_eq!(url_http("https://example.com:8443/x").unwrap(), "https://example.com:8443/x");
        assert_eq!(url_http("  https://example.com/a\\b  ").unwrap(), "https://example.com/a/b");
        assert_eq!(url_http("http:example.com").unwrap(), "http://example.com/");
        assert_eq!(url_http("https://user:pw@host/").unwrap(), "https://user:pw@host/");
        assert_eq!(url_http("https://[::1]:3000/").unwrap(), "https://[::1]:3000/");
        assert_eq!(url_http("https://example.com/ü").unwrap(), "https://example.com/%C3%BC");
        assert_eq!(message(url_http("")), "Invalid URL");
        assert_eq!(message(url_http("not a link")), "Invalid URL");
        assert_eq!(message(url_http("example.com")), "Invalid URL");
        assert_eq!(message(url_http("http://")), "Invalid URL");
        assert_eq!(message(url_http("http://exa mple.com")), "Invalid URL");
        assert_eq!(message(url_http("http://example.com:99999")), "Invalid URL");
        assert_eq!(message(url_http("http://example.com:8o")), "Invalid URL");
        assert_eq!(message(url_http("file:///C:/Windows")), "Only http(s) links can be opened");
        assert_eq!(message(url_http("javascript:alert(1)")), "Only http(s) links can be opened");
        assert_eq!(message(url_http("mailto:a@b.c")), "Only http(s) links can be opened");
        assert_eq!(message(url_http(r"C:\Windows")), "Only http(s) links can be opened");
    }

    #[test]
    fn places() {
        assert_eq!(place(&Value::Null), None);
        assert_eq!(place(&json!("g1")), None);
        assert_eq!(place(&json!([])), Some(Place::default()));
        assert_eq!(place(&json!({ "groupId": "g1", "afterId": "s1" })), Some(Place { group_id: Some("g1".into()), after_id: Some("s1".into()) }));
        let long = "x".repeat(65);
        assert_eq!(place(&json!({ "groupId": long, "afterId": 5 })), Some(Place::default()));
        assert_eq!(place(&json!({ "groupId": "" })), Some(Place::default()));
    }

    #[test]
    fn session_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let input = session_input(&json!({ "profileId": "p1", "cwd": cwd, "resume": "last", "place": { "groupId": "g2" } })).unwrap();
        assert_eq!(input.profile_id, "p1");
        assert_eq!(input.cwd, cwd);
        assert_eq!(input.resume, Some(ResumeMode::Last));
        assert_eq!(input.place, Some(Place { group_id: Some("g2".into()), after_id: None }));
        let other = session_input(&json!({ "profileId": "p1", "cwd": cwd, "resume": "never" })).unwrap();
        assert_eq!(other.resume, None);
        assert_eq!(other.place, None);
        // the folder is checked before the profile
        assert_eq!(message(session_input(&json!({}))), "Invalid folder");
        assert_eq!(message(session_input(&json!({ "cwd": cwd }))), "Invalid profile");
        assert_eq!(message(session_input(&json!({ "cwd": cwd, "profileId": 3 }))), "Invalid profile");
        let gone = dir.path().join("gone");
        let gone = gone.to_str().unwrap();
        assert_eq!(message(session_input(&json!({ "cwd": gone, "profileId": "p1" }))), format!("Folder not found: {gone}"));
    }

    #[test]
    fn refs() {
        let skill = skill_ref(&json!({ "rootId": "p-0123456789ab", "dirName": "review" })).unwrap();
        assert_eq!(skill, SkillRef { root_id: "p-0123456789ab".into(), dir_name: "review".into() });
        assert_eq!(message(skill_ref(&json!({ "dirName": "review" }))), "Invalid folder");
        assert_eq!(message(skill_ref(&json!({ "rootId": "p-0123456789ab" }))), "Invalid skill");
        assert_eq!(message(skill_ref(&json!(null))), "Invalid folder");
        assert_eq!(message(skill_ref(&json!({ "rootId": "p-0123456789ab", "dirName": "d".repeat(256) }))), "Invalid skill");

        let agent = agent_ref(&json!({ "rootId": "j-0123456789ab", "file": "team/qa.md" })).unwrap();
        assert_eq!(agent.file, "team/qa.md");
        assert_eq!(message(agent_ref(&json!({ "rootId": "j-0123456789ab", "file": "" }))), "Invalid agent");

        let memory = memory_ref(&json!({ "folderId": "m-0123456789ab", "file": "x.md" })).unwrap();
        assert_eq!(memory.folder_id, "m-0123456789ab");
        assert_eq!(message(memory_ref(&json!({ "folderId": "p-0123456789ab", "file": "x.md" }))), "Invalid folder");
        assert_eq!(message(memory_ref(&json!({ "folderId": "m-0123456789ab" }))), "Invalid memory");

        let instruction = instruction_ref(&json!({ "scopeId": "a-0123456789ab", "file": "CLAUDE.md" })).unwrap();
        assert_eq!(instruction.scope_id, "a-0123456789ab");
        assert_eq!(message(instruction_ref(&json!({ "scopeId": "a-0123456789ab", "file": "f".repeat(401) }))), "Invalid file");
    }

    #[test]
    fn instruction_targets() {
        assert_eq!(instruction_target(&json!({ "rule": "tests" })).unwrap(), InstructionTarget::Rule("tests".into()));
        assert_eq!(instruction_target(&json!({ "file": "CLAUDE.md" })).unwrap(), InstructionTarget::File("CLAUDE.md".into()));
        assert_eq!(message(instruction_target(&json!({ "rule": "" }))), "Invalid name");
        assert_eq!(message(instruction_target(&json!({ "rule": 5 }))), "Invalid file");
        assert_eq!(message(instruction_target(&json!({ "file": "f".repeat(65) }))), "Invalid file");
    }

    #[test]
    fn memory_create_inputs() {
        let ok = memory_create_input(&json!({ "name": "ui-language", "title": "", "description": "", "type": "user" })).unwrap();
        assert_eq!(ok.name, "ui-language");
        assert_eq!(ok.r#type, "user");
        assert_eq!(message(memory_create_input(&json!({ "title": "t", "description": "d", "type": "user" }))), "Invalid name");
        assert_eq!(message(memory_create_input(&json!({ "name": "n", "description": "d", "type": "user" }))), "Invalid title");
        assert_eq!(message(memory_create_input(&json!({ "name": "n", "title": "t".repeat(121), "description": "d", "type": "user" }))), "Invalid title");
        assert_eq!(
            message(memory_create_input(&json!({ "name": "n", "title": "t", "description": "d".repeat(301), "type": "user" }))),
            "Invalid description"
        );
        assert_eq!(message(memory_create_input(&json!({ "name": "n", "title": "t", "description": "d", "type": "" }))), "Invalid type");
    }

    #[test]
    fn start_run_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let input = start_run_input(&json!({ "workflowId": "w1", "cwd": cwd, "task": "" })).unwrap();
        assert_eq!(input, StartRunInput { workflow_id: "w1".into(), cwd: cwd.into(), task: String::new() });
        assert_eq!(message(start_run_input(&json!({ "workflowId": "w1", "cwd": cwd }))), "Invalid task");
        assert_eq!(message(start_run_input(&json!({ "workflowId": "w1", "cwd": cwd, "task": "t".repeat(100_001) }))), "Invalid task");
        assert_eq!(message(start_run_input(&json!({ "cwd": cwd, "task": "x" }))), "Invalid workflow");
        assert_eq!(message(start_run_input(&json!({ "workflowId": "w1", "task": "x" }))), "Invalid folder");
    }

    #[test]
    fn transfer_modes() {
        assert_eq!(transfer_mode("copy").unwrap(), TransferMode::Copy);
        assert_eq!(transfer_mode("move").unwrap(), TransferMode::Move);
        assert_eq!(message(transfer_mode("link")), "Invalid mode");
    }

    #[test]
    fn profile_patches() {
        let patch = profile_patch(&json!({ "name": "Work", "color": 5, "shareMemory": true, "slug": "x" }));
        assert_eq!(patch, ProfilePatch { name: Some("Work".into()), color: None, share_memory: Some(true) });
        assert_eq!(profile_patch(&Value::Null), ProfilePatch::default());
    }

    #[test]
    fn settings_patches() {
        let patch = settings_patch(&json!({
            "claudePath": "  C:\\bin\\claude.exe ",
            "fontSize": 30.4,
            "restoreTabs": false,
            "notifications": "yes",
            "limitAlerts": true,
            "autoSwitch": true,
            "tray": false,
            "openAtLogin": true,
            "explorerMenu": true,
            "trayHintShown": true,
            "skillFolders": ["C:\\x"],
            "tabStrip": "side"
        }));
        assert_eq!(
            patch,
            SettingsPatch {
                claude_path: Some(Some("C:\\bin\\claude.exe".into())),
                font_size: Some(28),
                restore_tabs: Some(false),
                notifications: None,
                limit_alerts: Some(true),
                auto_switch: Some(true),
                tray: Some(false),
                open_at_login: Some(true),
                tab_strip: Some(TabStrip::Side),
            }
        );
        // a claudePath that says nothing clears the setting; a missing one leaves it
        assert_eq!(settings_patch(&json!({ "claudePath": "   " })).claude_path, Some(None));
        assert_eq!(settings_patch(&json!({ "claudePath": null })).claude_path, Some(None));
        assert_eq!(settings_patch(&json!({ "fontSize": 12 })).claude_path, None);
        assert_eq!(settings_patch(&json!({ "fontSize": "12" })).font_size, None);
        assert_eq!(settings_patch(&json!(42)), SettingsPatch::default());

        let mut settings = crate::types::AppSettings { claude_path: Some("old".into()), tray: Some(true), ..Default::default() };
        settings_patch(&json!({ "claudePath": "", "fontSize": 15, "tray": false })).apply_to(&mut settings);
        assert_eq!(settings.claude_path, None);
        assert_eq!(settings.font_size, 15);
        assert_eq!(settings.tray, Some(false));
        assert_eq!(settings.restore_tabs, None);
    }
}
