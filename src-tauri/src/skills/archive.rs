//! Skills as one file (`src/main/skills/archive.ts`): a zip whose single top folder is the skill,
//! as Anthropic's own packager writes it and as claude.ai takes it (`.zip` or `.skill`).

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use mc_core::error::{AppError, AppResult};
use mc_core::frontmatter::{field_text, first_body_line, name_of, parse_doc};
use mc_core::jsstr::len16;
use mc_core::skills::{inspect_skill, is_device_name, new_name_problem, rename_in_text, slugify_skill_name};
use mc_core::text_file::{decode, encode};
use mc_core::types::{ImportPreview, IssueLevel, IssueScope, SkillFile, SkillIssue, SKILL_LIMITS};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::skills::files::{place_folder, WALK_LIMIT};

pub struct ArchiveLimits {
    /// The archive file itself.
    pub archive_bytes: u64,
    pub entries: usize,
    /// Everything unpacked, as the archive declares it.
    pub total_bytes: u64,
    pub file_bytes: u64,
    pub depth: usize,
    pub path_chars: usize,
}

pub const ARCHIVE_LIMITS: ArchiveLimits =
    ArchiveLimits { archive_bytes: 50 * 1024 * 1024, entries: 2000, total_bytes: 100 * 1024 * 1024, file_bytes: 20 * 1024 * 1024, depth: 12, path_chars: 180 };

/// What the packager leaves out: what tools fill by themselves, and the skill's own test runs.
fn left_out(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts.last().copied().unwrap_or_default();
    if parts.iter().any(|part| matches!(*part, "__pycache__" | "node_modules" | ".git")) {
        return true;
    }
    if parts.len() > 1 && parts[0] == "evals" {
        return true;
    }
    name == ".DS_Store" || name.ends_with(".pyc")
}

fn files_of(dir: &Path, sub: &str, found: &mut Vec<String>) -> std::io::Result<()> {
    let at = if sub.is_empty() { dir.to_path_buf() } else { dir.join(sub) };
    for entry in fs::read_dir(at)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else { continue };
        let rel = if sub.is_empty() { name } else { format!("{sub}/{name}") };
        if left_out(&rel) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files_of(dir, &rel, found)?;
        } else if kind.is_file() {
            found.push(rel);
        }
    }
    Ok(())
}

fn zip_failed(err: impl std::fmt::Display) -> AppError {
    AppError::io(format!("The archive could not be written ({err})."))
}

/// `packSkill`: packs a skill folder under one top folder `name`, with deflate at level 6. Links
/// inside it are not followed.
pub fn pack_skill(dir: &Path, name: &str) -> AppResult<Vec<u8>> {
    let mut files = Vec::new();
    files_of(dir, "", &mut files)?;
    if files.len() > ARCHIVE_LIMITS.entries {
        return Err(AppError::refused(format!("The skill has more than {} files.", ARCHIVE_LIMITS.entries)));
    }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).compression_level(Some(6));
    let mut total = 0u64;
    for rel in files {
        let bytes = fs::read(dir.join(&rel))?;
        total += bytes.len() as u64;
        if total > ARCHIVE_LIMITS.total_bytes {
            return Err(AppError::refused("The skill is too large to pack (more than 100 MB)."));
        }
        zip.start_file(format!("{name}/{rel}"), options).map_err(zip_failed)?;
        zip.write_all(&bytes)?;
    }
    Ok(zip.finish().map_err(zip_failed)?.into_inner())
}

/// `entryProblem`: why a path inside an archive is not written to disk, or None when it may be.
pub fn entry_problem(entry: &str) -> Option<String> {
    if len16(entry) > ARCHIVE_LIMITS.path_chars {
        return Some("a path is too long".to_owned());
    }
    let b = entry.as_bytes();
    if entry.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
        return Some("a path is absolute".to_owned());
    }
    let parts: Vec<&str> = entry.split('/').collect();
    if parts.len() > ARCHIVE_LIMITS.depth {
        return Some("folders are nested too deep".to_owned());
    }
    let quoted = |part: &str| serde_json::to_string(part).unwrap_or_default();
    for part in parts {
        if matches!(part, "" | "." | "..") {
            return Some("a path leaves the skill folder".to_owned());
        }
        if part.contains(['<', '>', ':', '"', '|', '?', '*', '\\']) || part.chars().any(|c| (c as u32) < 32) {
            return Some(format!("a name has characters that are not allowed: {}", quoted(part)));
        }
        if part.ends_with(['.', ' ']) {
            return Some(format!("a name ends with a dot or a space: {}", quoted(part)));
        }
        if is_device_name(part.split('.').next().unwrap_or_default()) {
            return Some(format!("\"{part}\" is a name Windows keeps for a device"));
        }
    }
    None
}

/// An archive that was read and checked: the one skill in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedArchive {
    /// The name the archive suggests for the skill.
    pub name: String,
    /// The skill's files by their path inside the skill folder, in the order of the archive.
    pub files: Vec<(String, Vec<u8>)>,
    /// Its SKILL.md as text, when that can be read.
    pub text: Option<String>,
}

fn unusable(message: impl Into<String>) -> AppError {
    AppError::invalid(message)
}

fn unreadable(err: impl std::fmt::Display) -> AppError {
    unusable(format!("This is not a zip archive that can be read ({err})."))
}

/// The files of a zip by their raw names, after the limits were checked from what the archive
/// says about itself and before anything is unpacked. A later entry of the same name takes the
/// place of the first.
fn unzip(bytes: &[u8]) -> AppResult<Vec<(String, Vec<u8>)>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(unreadable)?;
    let mut wanted: Vec<(usize, String, u64)> = Vec::new();
    let mut declared = 0u64;
    for index in 0..zip.len() {
        let file = zip.by_index_raw(index).map_err(unreadable)?;
        let name = file.name().to_owned();
        if name.ends_with('/') {
            continue;
        }
        if wanted.len() >= ARCHIVE_LIMITS.entries {
            return Err(unusable(format!("The archive holds more than {} files.", ARCHIVE_LIMITS.entries)));
        }
        let size = file.size();
        if size > ARCHIVE_LIMITS.file_bytes {
            return Err(unusable(format!("\"{name}\" in the archive is larger than 20 MB.")));
        }
        declared += size;
        if declared > ARCHIVE_LIMITS.total_bytes {
            return Err(unusable("The archive unpacks to more than 100 MB."));
        }
        wanted.push((index, name, size));
    }
    let mut unpacked: Vec<(String, Vec<u8>)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (index, name, size) in wanted {
        let file = zip.by_index(index).map_err(unreadable)?;
        let mut data = Vec::with_capacity(size as usize);
        // never more than the archive declared
        file.take(size + 1).read_to_end(&mut data).map_err(unreadable)?;
        if data.len() as u64 != size {
            return Err(unreadable(format!("\"{name}\" does not have the size the archive gives")));
        }
        match at.get(&name) {
            Some(&slot) => unpacked[slot].1 = data,
            None => {
                at.insert(name.clone(), unpacked.len());
                unpacked.push((name, data));
            }
        }
    }
    Ok(unpacked)
}

/// `readArchive`: reads an archive and takes out the one skill in it. Fails with the reason when
/// the archive cannot be used; nothing is written anywhere. `file_name` is the archive's own
/// name, for a skill that sits at the top of its archive without a folder.
pub fn read_archive(bytes: &[u8], file_name: &str) -> AppResult<CheckedArchive> {
    if bytes.len() as u64 > ARCHIVE_LIMITS.archive_bytes {
        return Err(unusable("The archive is larger than 50 MB."));
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut folded: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (raw, data) in unzip(bytes)? {
        let entry = raw.replace('\\', "/");
        // what macOS adds to every archive it makes
        if entry.starts_with("__MACOSX/") || entry.rsplit('/').next() == Some(".DS_Store") {
            continue;
        }
        if let Some(problem) = entry_problem(&entry) {
            return Err(unusable(format!("The archive cannot be used: {problem}.")));
        }
        if !folded.insert(entry.to_lowercase()) {
            return Err(unusable(format!("The archive holds \"{entry}\" twice in different spellings of upper and lower case.")));
        }
        entries.push((entry, data));
    }

    let mut tops: Vec<&str> = Vec::new();
    for (entry, _) in &entries {
        let top = entry.split_once('/').map_or("", |(top, _)| top);
        if !tops.contains(&top) {
            tops.push(top);
        }
    }
    let has = |path: &str| entries.iter().any(|(entry, _)| entry == path);
    let with_skill: Vec<&str> = tops.iter().copied().filter(|top| if top.is_empty() { has("SKILL.md") } else { has(&format!("{top}/SKILL.md")) }).collect();
    let top = match with_skill.as_slice() {
        [] => return Err(unusable("The archive has no SKILL.md at its top or in its one top folder.")),
        [top] => (*top).to_owned(),
        several => {
            let named: Vec<&str> = several.iter().map(|t| if t.is_empty() { "one at the top" } else { t }).collect();
            return Err(unusable(format!("The archive holds several skills ({}). Bring them in one by one.", named.join(", "))));
        }
    };
    if !top.is_empty() && tops.len() > 1 {
        return Err(unusable("The archive holds more than the one skill folder."));
    }

    let files: Vec<(String, Vec<u8>)> =
        entries.into_iter().map(|(entry, data)| if top.is_empty() { (entry, data) } else { (entry[top.len() + 1..].to_owned(), data) }).collect();
    let skill_md = files.iter().find(|(rel, _)| rel == "SKILL.md").map(|(_, data)| data.as_slice()).unwrap_or_default();
    let text = decode(skill_md, SKILL_LIMITS.file_bytes).ok().map(|d| d.text);
    let in_file = text.as_deref().and_then(|t| name_of(&parse_doc(t)));
    let base = Path::new(file_name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = base.to_lowercase();
    let stem = if lower.ends_with(".zip") {
        &base[..base.len() - 4]
    } else if lower.ends_with(".skill") {
        &base[..base.len() - 6]
    } else {
        &base
    };
    let from_file = slugify_skill_name(stem);
    let in_file = in_file.unwrap_or_default();
    let wishes = [top.as_str(), in_file.as_str(), from_file.as_str()];
    let name = match wishes.iter().find(|wish| !wish.is_empty() && new_name_problem(wish).is_none()) {
        Some(wish) => (*wish).to_owned(),
        None => {
            let fallback = slugify_skill_name(if !top.is_empty() { &top } else { &in_file });
            if fallback.is_empty() {
                "imported-skill".to_owned()
            } else {
                fallback
            }
        }
    };
    Ok(CheckedArchive { name, files, text })
}

/// `listArchive`: what an archive would bring in, for showing before it is done; and whether there
/// are more files than `limit`.
pub fn list_archive(archive: &CheckedArchive, limit: usize) -> (Vec<SkillFile>, bool) {
    let all: Vec<SkillFile> = archive.files.iter().filter(|(rel, _)| rel != "SKILL.md").map(|(rel, data)| SkillFile { path: rel.clone(), size: data.len() as u64 }).collect();
    let more = all.len() > limit;
    (all.into_iter().take(limit).collect(), more)
}

/// What the window shows of an archive before it is brought in (`pickImport`).
pub fn preview_of(archive: &CheckedArchive, token: String) -> ImportPreview {
    let (files, more_files) = list_archive(archive, WALK_LIMIT);
    let (description, issues) = match &archive.text {
        Some(text) => {
            let doc = parse_doc(text);
            let described = field_text(&doc, "description", false);
            let described = described.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}');
            let description = if described.is_empty() { first_body_line(doc.body) } else { described.to_owned() };
            (description, inspect_skill(&doc, &archive.name).into_iter().filter(|i| i.scope == IssueScope::Claude).collect())
        }
        None => (
            String::new(),
            vec![SkillIssue { level: IssueLevel::Warning, scope: IssueScope::Claude, message: "Its SKILL.md is not text this app can read.".to_owned(), clash: false }],
        ),
    };
    ImportPreview { token, name: archive.name.clone(), description, files, more_files, issues }
}

/// `unpackSkill`: writes a checked archive as a new skill folder `name` in `root_dir`, the name in
/// its SKILL.md made `name` as well: under another name than its own it must not answer to the
/// old one.
pub fn unpack_skill(archive: &CheckedArchive, root_dir: &Path, name: &str) -> AppResult<()> {
    place_folder(root_dir, name, |built| {
        for (rel, data) in &archive.files {
            let target = rel.split('/').fold(built.to_path_buf(), |p, part| p.join(part));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, data)?;
        }
        let Some(text) = &archive.text else { return Ok(()) };
        let renamed = rename_in_text(text, &archive.name, name, true);
        if renamed == *text {
            return Ok(());
        }
        let file = built.join("SKILL.md");
        if let Ok(decoded) = decode(&fs::read(&file)?, SKILL_LIMITS.file_bytes) {
            fs::write(&file, encode(&renamed, decoded.eol, decoded.bom))?;
        }
        Ok(())
    })?;
    Ok(())
}
