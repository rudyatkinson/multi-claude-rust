//! Where the Library looks for skills and for subagents (`src/main/skills/roots.ts`): both lie in
//! a `skills` or `agents` folder of an account or of a project.

use std::fs;
use std::path::{Component, Path, PathBuf};

use mc_core::types::{SkillRoot, SkillRootKind};
use sha1::{Digest, Sha1};

/// How far above a folder its project's skills are looked for.
const MAX_UP: usize = 6;

/// Which kind of folder: `.claude/skills` or `.claude/agents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootSub {
    Skills,
    Agents,
}

impl RootSub {
    /// The folder's name.
    pub fn as_str(self) -> &'static str {
        match self {
            RootSub::Skills => "skills",
            RootSub::Agents => "agents",
        }
    }
}

/// A folder that holds skills or subagents, with its real path; what the window sees of it is
/// [`RootSpec::shown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSpec {
    /// `p-` (an account's) or `j-` (a project's) and 12 hex digits of the real path's sha1.
    pub id: String,
    pub kind: SkillRootKind,
    /// The skills (or agents) folder itself.
    pub dir: PathBuf,
    /// The folder with links resolved; what `id` stands for.
    pub real: PathBuf,
    /// Personal: the accounts whose folder this is (several when they share it).
    pub profile_ids: Vec<String>,
    /// Project: the folder whose `.claude/skills` this is.
    pub folder: Option<PathBuf>,
    /// The folder is there. A project the user added may not have one yet.
    pub exists: bool,
    /// Project: the user added the folder to the list by hand.
    pub pinned: bool,
}

impl RootSpec {
    /// The root as the window sees it: without the resolved path.
    pub fn shown(&self) -> SkillRoot {
        SkillRoot {
            id: self.id.clone(),
            kind: self.kind,
            dir: self.dir.to_string_lossy().into_owned(),
            profile_ids: self.profile_ids.clone(),
            folder: self.folder.as_ref().map(|f| f.to_string_lossy().into_owned()),
            exists: self.exists,
            pinned: self.pinned.then_some(true),
        }
    }
}

/// What [`find_roots`] looks at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RootInputs {
    /// The accounts, with the Claude Code folder each one uses: `(profile id, config dir)`.
    pub profiles: Vec<(String, PathBuf)>,
    /// Folders worth looking at: where sessions run, what was opened lately.
    pub folders: Vec<String>,
    /// Folders the user added to the list by hand; they stay in it without skills too.
    pub pinned: Vec<String>,
}

fn is_dir(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_dir())
}

/// `path.isAbsolute` of Node for this platform: on Windows a path rooted without a drive (`\foo`)
/// counts as well.
pub fn is_absolute_js(p: &str) -> bool {
    let b = p.as_bytes();
    if cfg!(windows) {
        let sep = |c: u8| c == b'/' || c == b'\\';
        b.first().is_some_and(|&c| sep(c)) || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && sep(b[2]))
    } else {
        b.first() == Some(&b'/')
    }
}

/// `path.resolve(p)`: absolute, with `.` and `..` worked out on the text.
pub fn resolve(p: &Path) -> PathBuf {
    let absolute = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A path without Windows' `\\?\` prefix, which `canonicalize` puts in front: the spelling Node's
/// `realpathSync` gives and every program takes.
pub fn plain_path(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = p.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            let b = rest.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                return PathBuf::from(rest);
            }
        }
    }
    p
}

/// `realOf`: the folder a path really is, also when it is reached through a link or does not
/// exist yet (then it is the path made absolute).
pub fn real_of(p: &Path) -> PathBuf {
    fs::canonicalize(p).map(plain_path).unwrap_or_else(|_| resolve(p))
}

/// How paths are compared: without regard to case on Windows.
pub fn path_key(real: &Path) -> String {
    let text = real.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

fn id_of(prefix: &str, real: &Path) -> String {
    let digest = hex::encode(Sha1::digest(path_key(real).as_bytes()));
    format!("{prefix}-{}", &digest[..12])
}

/// `projectDirs`: the skills (or agents) folders of a project as Claude Code finds them from
/// `folder`: the folder's own and those of the folders above it, up to the top of the repository.
/// `(project folder, its .claude/<sub>)` for each that is there.
fn project_dirs(folder: &Path, sub: RootSub) -> Vec<(PathBuf, PathBuf)> {
    let start = resolve(folder);
    // the folders from this one up to the top of its repository; outside a repository only itself,
    // or the walk would end in the home folder and call the personal skills a project
    let mut chain = vec![start.clone()];
    let mut in_repo = start.join(".git").exists();
    let mut at = start.clone();
    let mut level = 0;
    while !in_repo && level < MAX_UP {
        let Some(up) = at.parent().map(Path::to_path_buf) else { break };
        chain.push(up.clone());
        in_repo = up.join(".git").exists();
        at = up;
        level += 1;
    }
    let looked = if in_repo { chain } else { vec![start] };
    looked
        .into_iter()
        .map(|at| {
            let dir = at.join(".claude").join(sub.as_str());
            (at, dir)
        })
        .filter(|(_, dir)| is_dir(dir))
        .collect()
}

/// `findRoots`: the folders that hold skills, or subagents with [`RootSub::Agents`]. Accounts
/// whose folder is one and the same folder (the others link to the main account's) make one
/// personal root; a project root is a `.claude/skills` folder. A root's id stands for its real
/// path, so the window never names a path itself.
pub fn find_roots(sub: RootSub, inputs: &RootInputs) -> Vec<RootSpec> {
    let mut roots: Vec<RootSpec> = Vec::new();
    let seen = |roots: &[RootSpec], real: &Path| -> Option<usize> {
        let key = path_key(real);
        roots.iter().position(|r| path_key(&r.real) == key)
    };

    for (id, config_dir) in &inputs.profiles {
        let dir = config_dir.join(sub.as_str());
        let real = real_of(&dir);
        if let Some(known) = seen(&roots, &real) {
            roots[known].profile_ids.push(id.clone());
            continue;
        }
        roots.push(RootSpec {
            id: id_of("p", &real),
            kind: SkillRootKind::Personal,
            exists: is_dir(&dir),
            dir,
            real,
            profile_ids: vec![id.clone()],
            folder: None,
            pinned: false,
        });
    }

    let add_project = |roots: &mut Vec<RootSpec>, folder: PathBuf, dir: PathBuf, pinned: bool| {
        let real = real_of(&dir);
        if let Some(known) = seen(roots, &real) {
            // the home folder's skills are the personal ones; a folder that is pinned stays marked
            if roots[known].kind == SkillRootKind::Project && pinned {
                roots[known].pinned = true;
            }
            return;
        }
        roots.push(RootSpec {
            id: id_of("j", &real),
            kind: SkillRootKind::Project,
            exists: is_dir(&dir),
            dir,
            real,
            profile_ids: Vec::new(),
            folder: Some(folder),
            pinned,
        });
    };

    for folder in &inputs.pinned {
        if !is_absolute_js(folder) {
            continue;
        }
        let at = resolve(Path::new(folder));
        // shown without skills as well, so that the first one can be made there
        let dir = at.join(".claude").join(sub.as_str());
        add_project(&mut roots, at, dir, true);
        for (found, dir) in project_dirs(Path::new(folder), sub) {
            add_project(&mut roots, found, dir, false);
        }
    }
    for folder in &inputs.folders {
        if !is_absolute_js(folder) {
            continue;
        }
        for (found, dir) in project_dirs(Path::new(folder), sub) {
            add_project(&mut roots, found, dir, false);
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    //! The `findRoots` tests of `test/skillFiles.test.ts` and `test/textFile.test.ts`.

    use super::*;
    use crate::library::test_support::link;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let real = real_of(dir.path());
        (dir, real)
    }

    fn mkdir(p: &Path) {
        fs::create_dir_all(p).unwrap();
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn makes_one_personal_root_of_accounts_that_share_a_skills_folder() {
        let (_guard, tmp) = tmp();
        let main = tmp.join("main");
        let elif = tmp.join("elif");
        let solo = tmp.join("solo");
        mkdir(&main.join("skills"));
        mkdir(&elif);
        link(&main.join("skills"), &elif.join("skills"));
        mkdir(&solo.join("skills"));
        let roots = find_roots(
            RootSub::Skills,
            &RootInputs {
                profiles: vec![("main".into(), main.clone()), ("elif".into(), elif), ("solo".into(), solo), ("empty".into(), tmp.join("empty"))],
                folders: vec![],
                pinned: vec![],
            },
        );
        let shape: Vec<(SkillRootKind, String, bool)> = roots.iter().map(|r| (r.kind, r.profile_ids.join("+"), r.exists)).collect();
        assert_eq!(
            shape,
            [
                (SkillRootKind::Personal, "main+elif".to_owned(), true),
                (SkillRootKind::Personal, "solo".to_owned(), true),
                (SkillRootKind::Personal, "empty".to_owned(), false)
            ]
        );
        assert_eq!(roots[0].dir, main.join("skills"));
        let ids: std::collections::HashSet<&str> = roots.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        assert!(mc_core::validate::skill_root(&roots[0].id).is_ok(), "{}", roots[0].id);
    }

    #[test]
    fn finds_the_skills_of_a_project_from_a_folder_inside_it_up_to_the_top_of_the_repository() {
        let (_guard, tmp) = tmp();
        let repo = tmp.join("repo");
        mkdir(&repo.join(".git"));
        mkdir(&repo.join(".claude").join("skills"));
        mkdir(&repo.join("apps").join("web").join(".claude").join("skills"));
        // above the repository nothing is looked at
        mkdir(&tmp.join(".claude").join("skills"));
        let web = repo.join("apps").join("web");
        let roots = find_roots(RootSub::Skills, &RootInputs { profiles: vec![], folders: vec![s(&web), s(&repo), "relative/path".into()], pinned: vec![] });
        assert_eq!(roots.iter().map(|r| r.folder.clone()).collect::<Vec<_>>(), [Some(web), Some(repo)]);
        assert!(roots.iter().all(|r| r.kind == SkillRootKind::Project && r.exists && !r.pinned));
        assert!(roots.iter().all(|r| r.id.starts_with("j-")));
    }

    #[test]
    fn keeps_a_folder_the_user_added_also_before_it_has_skills() {
        let (_guard, tmp) = tmp();
        let plain = tmp.join("plain");
        mkdir(&plain);
        let roots = find_roots(RootSub::Skills, &RootInputs { profiles: vec![], folders: vec![s(&plain)], pinned: vec![s(&plain)] });
        assert_eq!(roots.len(), 1);
        let shown = roots[0].shown();
        assert_eq!(shown.kind, SkillRootKind::Project);
        assert_eq!(shown.folder, Some(s(&plain)));
        assert_eq!(shown.dir, s(&plain.join(".claude").join("skills")));
        assert_eq!((shown.exists, shown.pinned), (false, Some(true)));
        // a pinned folder that also turns up among the folders stays pinned
        let roots = find_roots(RootSub::Skills, &RootInputs { profiles: vec![], folders: vec![s(&plain)], pinned: vec![s(&plain), s(&plain)] });
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn does_not_list_the_home_folders_skills_a_second_time_as_a_project() {
        let (_guard, tmp) = tmp();
        let home = tmp.join("home");
        mkdir(&home.join(".claude").join("skills"));
        let roots = find_roots(
            RootSub::Skills,
            &RootInputs { profiles: vec![("main".into(), home.join(".claude"))], folders: vec![s(&home)], pinned: vec![s(&home)] },
        );
        assert_eq!(roots.iter().map(|r| r.kind).collect::<Vec<_>>(), [SkillRootKind::Personal]);
        // the window never sees the resolved path
        let json = serde_json::to_value(roots[0].shown()).unwrap();
        assert!(json.get("real").is_none() && json.get("folder").is_none() && json.get("pinned").is_none());
    }

    #[test]
    fn looks_in_the_agents_folders_with_ids_of_their_own() {
        let (_guard, tmp) = tmp();
        let main = tmp.join("main");
        let other = tmp.join("other");
        let project = tmp.join("proj");
        for dir in [main.join("agents"), main.join("skills"), other.clone(), project.join(".git"), project.join(".claude").join("agents"), project.join("src")] {
            mkdir(&dir);
        }
        link(&main.join("agents"), &other.join("agents"));
        let inputs = RootInputs { profiles: vec![("main".into(), main.clone()), ("p2".into(), other)], folders: vec![s(&project.join("src"))], pinned: vec![] };

        let agents = find_roots(RootSub::Agents, &inputs);
        let shape: Vec<(SkillRootKind, PathBuf, Vec<String>)> = agents.iter().map(|r| (r.kind, r.dir.clone(), r.profile_ids.clone())).collect();
        assert_eq!(
            shape,
            [
                (SkillRootKind::Personal, main.join("agents"), vec!["main".to_owned(), "p2".to_owned()]),
                (SkillRootKind::Project, project.join(".claude").join("agents"), vec![])
            ]
        );
        assert_eq!(agents[1].folder, Some(project));

        // the same accounts and folders, asked for skills: other folders, other ids
        let skills = find_roots(RootSub::Skills, &inputs);
        assert_eq!(skills.iter().map(|r| r.kind).collect::<Vec<_>>(), [SkillRootKind::Personal, SkillRootKind::Personal]);
        assert!(!skills.iter().any(|r| agents.iter().any(|a| a.id == r.id)));
    }

    #[test]
    fn resolves_paths_as_node_does() {
        assert!(!is_absolute_js("relative/path"));
        if cfg!(windows) {
            assert!(is_absolute_js(r"C:\code") && is_absolute_js(r"\code") && is_absolute_js("C:/code"));
            assert!(!is_absolute_js("C:code"));
            assert_eq!(resolve(Path::new(r"C:\code\app\..\lib\.\x")), PathBuf::from(r"C:\code\lib\x"));
            assert_eq!(plain_path(PathBuf::from(r"\\?\C:\code")), PathBuf::from(r"C:\code"));
            assert_eq!(plain_path(PathBuf::from(r"\\?\UNC\server\share")), PathBuf::from(r"\\server\share"));
        } else {
            assert!(is_absolute_js("/code"));
            assert_eq!(resolve(Path::new("/code/app/../lib/./x")), PathBuf::from("/code/lib/x"));
        }
        // a path that is not there is made absolute
        let (_guard, tmp) = tmp();
        assert_eq!(real_of(&tmp.join("missing").join("x")), tmp.join("missing").join("x"));
    }
}
