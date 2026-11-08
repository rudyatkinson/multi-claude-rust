//! Everything the skill manager does on disk (`src/main/skills/store.ts`, and the skill parts of
//! `src/main/index.ts`: files, imports, Ask Claude).
//!
//! A skill is always named by its root and its folder name; the folder name has to be one that
//! the root really lists, and the roots are found again for every call, so nothing that comes from
//! the window is ever used as a path as it stands.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mc_core::error::{AppError, AppResult};
use mc_core::skills::{is_reserved_skill_dir, name_clashes, new_name_problem, new_skill_text, rename_in_text, skill_key, Named};
use mc_core::text_file::{busy_message, decode, encode, read_text, retry_busy, save_text, write_file_atomic, TextRead, BUSY_TRIES, BUSY_WAIT};
use mc_core::types::{
    ImportPreview, Place, SaveOutcome, SaveResult, SessionInfo, SessionRequest, SkillDetail, SkillListing, SkillMoved, SkillRef, SkillRoot, SkillRootKind,
    TransferMode, SKILL_LIMITS,
};

use crate::claude_binary::is_shim;
use crate::library::roots::RootSpec;
use crate::library::trash::{delete_item, TrashFn};
use crate::skills::archive::{pack_skill, preview_of, read_archive, unpack_skill, CheckedArchive};
use crate::skills::assist::{start_assist, AssistInput};
use crate::skills::files::{copy_tree, entry_named, names_in, place_folder, walk_skill, WALK_LIMIT};
use crate::skills::scan::{find_skills, summarize};

const GONE: &str = "That skill is no longer there. The list may be out of date.";
const ROOT_GONE: &str = "That folder is no longer in the list.";
const FILE_GONE: &str = "That file is no longer there.";
/// How long a chosen archive waits to be brought in.
pub const IMPORT_KEPT: Duration = Duration::from_secs(15 * 60);
/// What opens with its own program when clicked; anything else, scripts above all, is only shown.
const SAFE_TO_OPEN: [&str; 7] = ["md", "txt", "json", "yaml", "yml", "toml", "csv"];

/// The folders that hold skills right now.
pub type RootsFn = Box<dyn Fn() -> Vec<RootSpec> + Send + Sync>;
/// The names of accounts, for telling whose personal skills a folder holds.
pub type NamesFn = Box<dyn Fn(&[String]) -> String + Send + Sync>;

/// What the service works with. The app's is [`SkillService::new`]; tests give their own.
pub struct SkillDeps {
    pub roots: RootsFn,
    /// Moves a folder to the system's bin. Fails when there is none for that drive.
    pub trash: TrashFn,
    pub account_names: NamesFn,
    /// Holds one folder per request to Claude about a skill.
    pub requests_dir: PathBuf,
}

/// An archive that was chosen and checked, until it is brought in: the window never names a path.
struct PendingImport {
    archive: CheckedArchive,
    at: Instant,
}

/// What Ask Claude is asked for, with what the app knows about it.
pub struct AssistRequest<'a> {
    pub r: &'a SkillRef,
    pub profile_id: &'a str,
    /// The account's name, None when there is no such account.
    pub profile_name: Option<&'a str>,
    pub request: &'a str,
    pub place: Option<Place>,
    /// The claude binary, None when it was not found.
    pub claude_path: Option<&'a str>,
}

/// A session that was opened about a skill.
pub struct Assisted {
    pub info: SessionInfo,
    /// The project it works in, to remember among the recent folders.
    pub project: Option<String>,
}

/// What the window can do with a file of a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    /// Open it with its own program.
    Open(PathBuf),
    /// Show it in the file manager: "opening" a script would run it.
    Reveal(PathBuf),
}

pub struct SkillService {
    deps: SkillDeps,
    pending: Mutex<HashMap<String, PendingImport>>,
}

impl SkillService {
    /// The app's service: the roots of the accounts and projects in sight, the system's bin, and
    /// `<root>/skill-requests` for Ask Claude, whose folders older than two weeks go at once.
    pub fn new(app: tauri::AppHandle, root: PathBuf) -> Self {
        let roots_app = app.clone();
        let names_app = app;
        let requests_dir = crate::paths::Paths::new(root).skill_requests_dir();
        let prune_dir = requests_dir.clone();
        std::thread::spawn(move || crate::skills::assist::prune_requests(&prune_dir, std::time::SystemTime::now()));
        Self::with_deps(SkillDeps {
            roots: Box::new(move || {
                crate::app::state(&roots_app).map(|state| crate::library::known_folders::library_roots(&state, crate::library::roots::RootSub::Skills)).unwrap_or_default()
            }),
            trash: crate::library::trash::system_trash_fn(),
            account_names: Box::new(move |ids| crate::app::state(&names_app).map(|state| crate::library::known_folders::account_names(&state, ids)).unwrap_or_default()),
            requests_dir,
        })
    }

    pub fn with_deps(deps: SkillDeps) -> Self {
        SkillService { deps, pending: Mutex::default() }
    }

    /// The folders that hold skills right now.
    pub fn roots(&self) -> Vec<RootSpec> {
        (self.deps.roots)()
    }

    /// A root by its id.
    pub fn root_of(&self, root_id: &str) -> AppResult<RootSpec> {
        self.roots().into_iter().find(|r| r.id == root_id).ok_or_else(|| AppError::refused(ROOT_GONE))
    }

    /// The root, the folder and whether it is a link, of an existing skill.
    fn locate(&self, r: &SkillRef) -> AppResult<(RootSpec, PathBuf, bool)> {
        let root = self.root_of(&r.root_id)?;
        let found = find_skills(&root.dir).into_iter().find(|s| s.dir_name == r.dir_name).ok_or_else(|| AppError::refused(GONE))?;
        let dir = root.dir.join(&found.dir_name);
        Ok((root, dir, found.linked))
    }

    /// The folder of a skill, for showing it in the file manager or packing it.
    pub fn dir_of(&self, r: &SkillRef) -> AppResult<PathBuf> {
        Ok(self.locate(r)?.1)
    }

    /// `skillFile`: a file of a skill, once the skill's own listing has it.
    pub fn file_of(&self, r: &SkillRef, file: &str) -> AppResult<PathBuf> {
        let dir = self.dir_of(r)?;
        if file != "SKILL.md" && !walk_skill(&dir, 5000).0.iter().any(|f| f.path == file) {
            return Err(AppError::refused(FILE_GONE));
        }
        Ok(file.split('/').fold(dir, |p, part| p.join(part)))
    }

    /// `openSkillFile`: what clicking a file of a skill does. Text opens with its program; anything
    /// else is shown in the file manager.
    pub fn file_action(&self, r: &SkillRef, file: &str) -> AppResult<FileAction> {
        let target = self.file_of(r, file)?;
        let ext = target.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        Ok(if SAFE_TO_OPEN.contains(&ext.as_str()) { FileAction::Open(target) } else { FileAction::Reveal(target) })
    }

    // ---------- reading ----------

    /// `list`: the roots as the window sees them and the skills of every one, with the clashes
    /// between their names.
    pub fn list(&self) -> SkillListing {
        let roots = self.roots();
        let mut skills: Vec<_> = roots.iter().flat_map(|root| find_skills(&root.dir).into_iter().map(move |skill| summarize(root, &skill))).collect();
        // the window has no use for the resolved paths
        let shown: Vec<SkillRoot> = roots.iter().map(RootSpec::shown).collect();
        let named: Vec<Named<'_>> = skills.iter().map(|s| Named { root_id: &s.root_id, dir_name: &s.dir_name, name: &s.name }).collect();
        let mut clashes = name_clashes(&named, &shown, |root| (self.deps.account_names)(&root.profile_ids));
        for skill in &mut skills {
            if let Some(issues) = clashes.remove(&skill_key(&skill.skill_ref())) {
                skill.issues.extend(issues);
            }
        }
        SkillListing { roots: shown, skills }
    }

    fn detail(r: &SkillRef, dir: &Path, linked: bool, on_disk: TextRead) -> SkillDetail {
        let (files, more_files) = walk_skill(dir, WALK_LIMIT);
        SkillDetail {
            root_id: r.root_id.clone(),
            dir_name: r.dir_name.clone(),
            text: on_disk.text,
            hash: on_disk.hash,
            files,
            more_files,
            path: dir.to_string_lossy().into_owned(),
            linked,
            read_only: on_disk.read_only,
        }
    }

    pub fn read(&self, r: &SkillRef) -> AppResult<SkillDetail> {
        let (_, dir, linked) = self.locate(r)?;
        let on_disk = read_text(&dir.join("SKILL.md"), SKILL_LIMITS.file_bytes)?;
        Ok(Self::detail(r, &dir, linked, on_disk))
    }

    /// Writes a SKILL.md. `base_hash` is the state the editor started from: when the file on disk
    /// is another one by now (a Claude session changed it), nothing is written and the result says
    /// so, unless `base_hash` is None, which overwrites.
    pub fn save(&self, r: &SkillRef, text: &str, base_hash: Option<&str>) -> AppResult<SaveResult> {
        let (_, dir, linked) = self.locate(r)?;
        Ok(match save_text(&dir.join("SKILL.md"), text, base_hash)? {
            SaveOutcome::Saved(on_disk) => SaveOutcome::Saved(Self::detail(r, &dir, linked, on_disk)),
            SaveOutcome::Conflict(on_disk) => SaveOutcome::Conflict(Self::detail(r, &dir, linked, on_disk)),
        })
    }

    // ---------- changing ----------

    /// Checks a name for a new folder in a root; `except` is a folder that may keep its own name.
    fn free_name(root: &RootSpec, name: &str, except: Option<&str>) -> AppResult<()> {
        if let Some(problem) = new_name_problem(name) {
            return Err(AppError::invalid(problem));
        }
        // a folder that is not there yet is made with its first skill
        let names = names_in(&root.dir);
        match entry_named(&names, name) {
            Some(taken) if Some(taken) != except => Err(AppError::refused(format!("There is already a skill named \"{taken}\" there."))),
            _ => Ok(()),
        }
    }

    pub fn create(&self, root_id: &str, name: &str, description: &str) -> AppResult<SkillRef> {
        let root = self.root_of(root_id)?;
        Self::free_name(&root, name, None)?;
        place_folder(&root.dir, name, |dir| Ok(fs::write(dir.join("SKILL.md"), new_skill_text(name, description))?))?;
        Ok(SkillRef { root_id: root_id.to_owned(), dir_name: name.to_owned() })
    }

    /// A copy of a skill under another name, in its own root or in `to_root_id`.
    fn copy_to(&self, r: &SkillRef, to_root_id: &str, name: &str) -> AppResult<SkillRef> {
        let (_, dir, _) = self.locate(r)?;
        let target = self.root_of(to_root_id)?;
        Self::free_name(&target, name, None)?;
        place_folder(&target.dir, name, |built| {
            copy_tree(&dir, built)?;
            let file = built.join("SKILL.md");
            let Ok(decoded) = decode(&fs::read(&file)?, SKILL_LIMITS.file_bytes) else { return Ok(()) };
            // a copy that kept the name in its file would answer to the name of its original
            if name == r.dir_name {
                return Ok(());
            }
            let text = rename_in_text(&decoded.text, &r.dir_name, name, true);
            if text != decoded.text {
                fs::write(&file, encode(&text, decoded.eol, decoded.bom))?;
            }
            Ok(())
        })?;
        Ok(SkillRef { root_id: to_root_id.to_owned(), dir_name: name.to_owned() })
    }

    pub fn duplicate(&self, r: &SkillRef, name: &str) -> AppResult<SkillRef> {
        self.copy_to(r, &r.root_id, name)
    }

    /// Renames a skill's folder; a name in the file that was the folder's name follows. The folder
    /// goes first: that is the step that can fail. When the name in the file cannot be changed
    /// after it, the result says so.
    pub fn rename(&self, r: &SkillRef, name: &str) -> AppResult<SkillMoved> {
        let (root, dir, _) = self.locate(r)?;
        if name == r.dir_name {
            return Ok(SkillMoved { r#ref: r.clone(), warning: None });
        }
        Self::free_name(&root, name, Some(&r.dir_name))?;
        let target = root.dir.join(name);
        retry_busy(|| fs::rename(&dir, &target), BUSY_TRIES, BUSY_WAIT).map_err(|err| busy_message(err, "Renaming"))?;
        let moved = SkillRef { root_id: r.root_id.clone(), dir_name: name.to_owned() };
        let file = target.join("SKILL.md");
        let renamed = (|| -> std::io::Result<()> {
            let Ok(decoded) = decode(&fs::read(&file)?, SKILL_LIMITS.file_bytes) else { return Ok(()) };
            let text = rename_in_text(&decoded.text, &r.dir_name, name, false);
            if text != decoded.text {
                write_file_atomic(&file, &encode(&text, decoded.eol, decoded.bom))?;
            }
            Ok(())
        })();
        let warning = renamed.err().map(|_| "The folder has its new name; the name inside SKILL.md could not be changed.".to_owned());
        Ok(SkillMoved { r#ref: moved, warning })
    }

    /// Puts a skill into the bin. A skill that is a link loses only the link.
    pub fn remove(&self, r: &SkillRef) -> AppResult<()> {
        let (_, dir, _) = self.locate(r)?;
        delete_item(&self.deps.trash, &dir, "skill")
    }

    /// Copies a skill into another root, or moves it there (a copy, then the original into the
    /// bin; when that fails the copy stands and the result says so).
    pub fn transfer(&self, r: &SkillRef, to_root_id: &str, mode: TransferMode, name: &str) -> AppResult<SkillMoved> {
        if to_root_id == r.root_id && name == r.dir_name {
            return Err(AppError::refused("The skill is in that folder already."));
        }
        if is_reserved_skill_dir(name) {
            return Err(AppError::invalid(format!("\"{name}\" is a name Claude Code keeps for itself.")));
        }
        let (_, _, linked) = self.locate(r)?;
        if mode == TransferMode::Move && linked {
            return Err(AppError::refused("This skill is a link to a folder elsewhere and cannot be moved. Copy it instead."));
        }
        let copy = self.copy_to(r, to_root_id, name)?;
        if mode == TransferMode::Copy {
            return Ok(SkillMoved { r#ref: copy, warning: None });
        }
        let warning = self.remove(r).err().map(|err| format!("The skill was copied, but the original is still there: {err}"));
        Ok(SkillMoved { r#ref: copy, warning })
    }

    // ---------- archives ----------

    /// The skill packed as a zip, for `skills:export`.
    pub fn pack(&self, r: &SkillRef) -> AppResult<Vec<u8>> {
        pack_skill(&self.dir_of(r)?, &r.dir_name)
    }

    fn lock_pending(&self) -> std::sync::MutexGuard<'_, HashMap<String, PendingImport>> {
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Checks a chosen archive and keeps it for [`import`](Self::import) under a token, for
    /// [`IMPORT_KEPT`]; what it holds, for showing.
    pub fn preview_import(&self, bytes: &[u8], file_name: &str) -> AppResult<ImportPreview> {
        let archive = read_archive(bytes, file_name)?;
        let token = uuid::Uuid::new_v4().to_string();
        let preview = preview_of(&archive, token.clone());
        let mut pending = self.lock_pending();
        pending.retain(|_, kept| kept.at.elapsed() <= IMPORT_KEPT);
        pending.insert(token, PendingImport { archive, at: Instant::now() });
        Ok(preview)
    }

    /// `importSkill`: brings a checked archive in as the skill `name` of a root. The archive stays
    /// under its token until that worked, so that another name can be tried.
    pub fn import(&self, token: &str, root_id: &str, name: &str) -> AppResult<SkillRef> {
        let kept = self.lock_pending().remove(token).filter(|kept| kept.at.elapsed() <= IMPORT_KEPT);
        let Some(kept) = kept else { return Err(AppError::refused("Choose the archive again.")) };
        let unpacked = (|| {
            if let Some(problem) = new_name_problem(name) {
                return Err(AppError::invalid(problem));
            }
            unpack_skill(&kept.archive, &self.root_of(root_id)?.dir, name)
        })();
        if let Err(err) = unpacked {
            self.lock_pending().insert(token.to_owned(), kept);
            return Err(err);
        }
        Ok(SkillRef { root_id: root_id.to_owned(), dir_name: name.to_owned() })
    }

    // ---------- Ask Claude ----------

    /// `assistSkill`: opens a session about a skill with `start`. Refused without a request,
    /// without claude or with its `.cmd` shim (the first message would pass through cmd.exe), for
    /// an account that does not exist or does not see a personal skill.
    pub fn assist(&self, ask: AssistRequest<'_>, start: impl FnOnce(&SessionRequest, Option<Place>) -> AppResult<SessionInfo>) -> AppResult<Assisted> {
        if ask.request.trim().is_empty() {
            return Err(AppError::invalid("Say what Claude should do with the skill."));
        }
        let claude = ask.claude_path.ok_or_else(|| AppError::refused(crate::app::launch::CLAUDE_NOT_FOUND))?;
        if is_shim(claude) {
            return Err(AppError::refused("Asking Claude needs the native claude executable, not the .cmd one. Set its path in Settings."));
        }
        let profile_name = ask.profile_name.ok_or_else(|| AppError::refused("Profile not found"))?;
        let root = self.root_of(&ask.r.root_id)?;
        if root.kind == SkillRootKind::Personal && !root.profile_ids.iter().any(|id| id == ask.profile_id) {
            return Err(AppError::refused(format!("\"{profile_name}\" has its own skills folder and does not see this skill.")));
        }
        let dir = self.dir_of(ask.r)?;
        let input = AssistInput { root: &root, dir_name: &ask.r.dir_name, dir: &dir, profile_id: ask.profile_id, request: ask.request, place: ask.place };
        let info = start_assist(&self.deps.requests_dir, input, start)?;
        let project = match (root.kind, &root.folder) {
            (SkillRootKind::Project, Some(folder)) => Some(folder.to_string_lossy().into_owned()),
            _ => None,
        };
        Ok(Assisted { info, project })
    }
}
