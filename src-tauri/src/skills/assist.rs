//! "Ask Claude" about a skill (`src/main/skills/assist.ts`): an ordinary session tab whose first
//! message points at a file with the request. It is the way workflow steps start: a command line
//! has a length limit and quoting rules, a file has neither.

use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use mc_core::error::{AppError, AppResult};
use mc_core::types::{LaunchOptions, Place, SessionInfo, SessionKind, SessionRequest, SkillRootKind};
use regex::Regex;

use crate::library::roots::RootSpec;

/// How long a request's folder is kept.
pub const KEEP_REQUESTS: Duration = Duration::from_secs(14 * 24 * 60 * 60);

static NOT_PLAIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{N} _.,()+\-]").expect("valid regex"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));

fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `assistPrompt`: what Claude is told: the user's request, and what a skill has to keep to.
pub fn assist_prompt(dir: &str, dir_name: &str, request: &str) -> String {
    [
        "You are asked to work on a Claude Code skill.".to_owned(),
        String::new(),
        format!("Skill folder: {dir}"),
        "Its instructions are in SKILL.md in that folder: a YAML frontmatter between two lines of \"---\", then Markdown. Other files in the folder support it."
            .to_owned(),
        String::new(),
        "What is asked:".to_owned(),
        String::new(),
        request.trim_matches(is_js_space).to_owned(),
        String::new(),
        "Keep to this:".to_owned(),
        "- The frontmatter stays valid YAML. A value that contains a colon followed by a space stands in quotes.".to_owned(),
        format!("- \"name: {dir_name}\" stays as it is; the folder has that name."),
        "- \"description\" says what the skill does and when to use it, in the third person and in at most 1024 characters. Claude decides by it whether the skill fits."
            .to_owned(),
        "- SKILL.md stays under 500 lines. Long reference material goes into files next to it, linked from SKILL.md.".to_owned(),
        "- Change only files in the skill folder.".to_owned(),
        String::new(),
        "When you are done, say in a few lines what you changed.".to_owned(),
        String::new(),
    ]
    .join("\n")
}

/// `assistMessage`: the message the session starts with. The name ends up inside quotes on a
/// command line, so only letters, digits and a few harmless marks of it stay.
pub fn assist_message(dir_name: &str, file: &str) -> String {
    let plain = NOT_PLAIN.replace_all(dir_name, " ");
    let plain = SPACES.replace_all(&plain, " ");
    let plain = plain.trim_matches(is_js_space);
    format!("Skill \"{plain}\": follow the instructions in @\"{file}\". If that file is shown to you only in part, read the rest of it before you start.")
}

/// What [`start_assist`] opens a session for.
pub struct AssistInput<'a> {
    pub root: &'a RootSpec,
    pub dir_name: &'a str,
    /// The skill's folder.
    pub dir: &'a Path,
    pub profile_id: &'a str,
    pub request: &'a str,
    pub place: Option<Place>,
}

/// `startAssist`: writes the request into `<requests_dir>/<8 hex>/request.md` and opens the
/// session with `start`. It works in the project for a project skill and in the skills folder for
/// a personal one, not in the skill's own folder: a session that sits in a folder keeps Windows
/// from renaming or deleting it, and every new folder would ask for trust again.
pub fn start_assist(
    requests_dir: &Path,
    input: AssistInput<'_>,
    start: impl FnOnce(&SessionRequest, Option<Place>) -> AppResult<SessionInfo>,
) -> AppResult<SessionInfo> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let request_dir = requests_dir.join(&token[..8]);
    let shown = request_dir.to_string_lossy().into_owned();
    if shown.contains(['"', '\r', '\n']) {
        return Err(AppError::refused("The folder that holds the app data has a name that cannot be handed to Claude."));
    }
    fs::create_dir_all(&request_dir)?;
    let file = request_dir.join("request.md");
    fs::write(&file, assist_prompt(&input.dir.to_string_lossy(), input.dir_name, input.request))?;
    let cwd = match (&input.root.kind, &input.root.folder) {
        (SkillRootKind::Project, Some(folder)) => folder.clone(),
        _ => input.root.real.clone(),
    };
    let request = SessionRequest {
        profile_id: input.profile_id.to_owned(),
        cwd: cwd.to_string_lossy().into_owned(),
        kind: SessionKind::Claude,
        resume: None,
        name: Some(format!("Skill: {}", input.dir_name)),
        launch: Some(LaunchOptions { add_dirs: Some(vec![shown]), ..LaunchOptions::default() }),
        prompt: Some(assist_message(input.dir_name, &file.to_string_lossy())),
    };
    start(&request, input.place)
}

/// `pruneRunFolders` of the requests: takes away the request folders older than
/// [`KEEP_REQUESTS`]. A folder that is gone already or in use stays for the next time.
pub fn prune_requests(requests_dir: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(requests_dir) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        let old = fs::metadata(&dir).and_then(|m| m.modified()).is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age > KEEP_REQUESTS));
        if old {
            let _ = fs::remove_dir_all(&dir).or_else(|_| fs::remove_file(&dir));
        }
    }
}

#[cfg(test)]
mod tests {
    //! The "asking Claude about a skill" tests of `test/skillArchive.test.ts`.

    use std::cell::RefCell;
    use std::path::PathBuf;

    use mc_core::types::SessionStatus;

    use super::*;

    fn root(kind: SkillRootKind, dir: &str, real: &str, folder: Option<&str>) -> RootSpec {
        RootSpec {
            id: if kind == SkillRootKind::Personal { "p-1" } else { "j-1" }.into(),
            kind,
            dir: dir.into(),
            real: real.into(),
            profile_ids: if kind == SkillRootKind::Personal { vec!["main".into()] } else { vec![] },
            folder: folder.map(PathBuf::from),
            exists: true,
            pinned: false,
        }
    }

    fn info(id: &str, request: &SessionRequest) -> SessionInfo {
        serde_json::from_value(serde_json::json!({
            "id": id, "profileId": request.profile_id, "kind": "claude", "cwd": request.cwd, "title": "t",
            "status": "running", "startedAt": 0
        }))
        .unwrap_or_else(|e| panic!("a SessionInfo: {e}"))
    }

    #[test]
    fn writes_the_request_and_the_rules_of_a_skill_into_a_file() {
        let prompt = assist_prompt(r"C:\home\.claude\skills\review", "review", "  Make the description say when to use it.  ");
        assert!(prompt.contains("Skill folder: C:\\home\\.claude\\skills\\review\n"));
        assert!(prompt.contains("What is asked:\n\nMake the description say when to use it.\n"));
        assert!(prompt.contains("\"name: review\" stays as it is"));
        assert!(prompt.contains("at most 1024 characters"));
        assert!(prompt.ends_with("what you changed.\n"));
    }

    #[test]
    fn names_only_the_file_on_the_command_line() {
        assert_eq!(
            assist_message("review", r"C:\data\skill-requests\ab12\request.md"),
            "Skill \"review\": follow the instructions in @\"C:\\data\\skill-requests\\ab12\\request.md\". If that file is shown to you only in part, read the rest of it before you start."
        );
        assert!(assist_message("odd\"name`$(x)", "f").contains("Skill \"odd name (x)\""));
        assert!(assist_message("İnceleme-ağ", "f").contains("Skill \"İnceleme-ağ\""));
    }

    #[test]
    fn opens_a_session_in_the_project_for_a_project_skill_and_in_the_skills_folder_for_a_personal_one() {
        let tmp = tempfile::tempdir().unwrap();
        let requests_dir = tmp.path().join("skill-requests");
        let personal = root(SkillRootKind::Personal, r"C:\links\skills", r"C:\home\.claude\skills", None);
        let project = root(SkillRootKind::Project, r"W:\app\.claude\skills", r"W:\app\.claude\skills", Some(r"W:\app"));
        let calls: RefCell<Vec<(SessionRequest, Option<Place>)>> = RefCell::new(Vec::new());
        let start = |request: &SessionRequest, place: Option<Place>| {
            calls.borrow_mut().push((request.clone(), place));
            Ok(info(&format!("tab{}", calls.borrow().len()), request))
        };

        let place = Place { group_id: Some("g1".into()), after_id: None };
        let dir = Path::new(r"W:\app\.claude\skills\review");
        let input = AssistInput { root: &project, dir_name: "review", dir, profile_id: "work", request: "Shorten it.", place: Some(place.clone()) };
        let opened = start_assist(&requests_dir, input, start).unwrap();
        assert_eq!(opened.id, "tab1");
        let (request, given) = calls.borrow()[0].clone();
        assert_eq!(given, Some(place));
        assert_eq!((request.profile_id.as_str(), request.cwd.as_str(), request.kind), ("work", r"W:\app", SessionKind::Claude));
        assert_eq!(request.name.as_deref(), Some("Skill: review"));
        let request_dir = PathBuf::from(&request.launch.as_ref().unwrap().add_dirs.as_ref().unwrap()[0]);
        assert_eq!(request_dir.parent(), Some(requests_dir.as_path()));
        assert_eq!(request.launch, Some(LaunchOptions { add_dirs: Some(vec![request_dir.to_string_lossy().into_owned()]), ..LaunchOptions::default() }));
        let file = request_dir.join("request.md");
        assert_eq!(request.prompt.as_deref(), Some(assist_message("review", &file.to_string_lossy()).as_str()));
        assert_eq!(fs::read_to_string(&file).unwrap(), assist_prompt(r"W:\app\.claude\skills\review", "review", "Shorten it."));
        // the request text is nowhere on the command line
        assert!(!request.prompt.as_deref().unwrap().contains("Shorten it."));
        assert_eq!(opened.status, SessionStatus::Running);

        let input = AssistInput { root: &personal, dir_name: "review", dir: Path::new(r"C:\links\skills\review"), profile_id: "main", request: "x", place: None };
        start_assist(&requests_dir, input, start).unwrap();
        let second = calls.borrow()[1].0.clone();
        assert_eq!(second.cwd, r"C:\home\.claude\skills");
        assert_ne!(second.launch.unwrap().add_dirs.unwrap()[0], request_dir.to_string_lossy());
    }

    #[test]
    fn refuses_a_data_folder_whose_name_cannot_be_quoted() {
        let tmp = tempfile::tempdir().unwrap();
        if cfg!(windows) {
            // a quote cannot be in a Windows path at all
            return;
        }
        let personal = root(SkillRootKind::Personal, "/s", "/s", None);
        let input = AssistInput { root: &personal, dir_name: "x", dir: Path::new("/s/x"), profile_id: "main", request: "x", place: None };
        let err = start_assist(&tmp.path().join("a\"b"), input, |_, _| panic!("no session")).unwrap_err();
        assert!(err.to_string().contains("cannot be handed to Claude"));
    }

    #[test]
    fn keeps_the_requests_of_the_last_two_weeks() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("ab12cd34")).unwrap();
        prune_requests(tmp.path(), SystemTime::now());
        assert!(tmp.path().join("ab12cd34").exists());
        prune_requests(tmp.path(), SystemTime::now() + KEEP_REQUESTS + Duration::from_secs(60));
        assert!(!tmp.path().join("ab12cd34").exists());
        prune_requests(&tmp.path().join("missing"), SystemTime::now());
    }
}
