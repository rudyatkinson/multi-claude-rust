//! Serde round trips of the types against JSON in the shapes the TypeScript wrote.

use std::fmt::Debug;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use super::*;

/// Reads `json` as `T`, writes it back and checks that nothing changed on the way.
fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(json: Value) -> T {
    let typed: T = serde_json::from_value(json.clone()).expect("deserializes");
    let back = serde_json::to_value(&typed).expect("serializes");
    assert_eq!(back, json, "the JSON changed on a round trip");
    let again: T = serde_json::from_value(back).expect("deserializes again");
    assert_eq!(again, typed);
    typed
}

fn profile_json() -> Value {
    json!({
        "id": "p-work",
        "name": "Work",
        "slug": "work",
        "color": "#5b9bd5",
        "isDefault": false,
        "configDir": "C:\\Users\\me\\.multi-claude\\profiles\\work",
        "share": { "dirs": ["skills"], "files": ["CLAUDE.md"], "localSettingsKeys": ["model"] },
        "emailHint": "me@example.com",
        "auth": "token",
        "tokenEnc": "djEwAAAA",
        "lastSync": { "at": 1731000000000_i64, "mainHashes": { "CLAUDE.md": "ab" }, "localHashes": {} },
        "account": {
            "loggedIn": true,
            "email": "me@example.com",
            "orgName": "Arvis",
            "subscriptionType": "max",
            "authMethod": "oauth_token",
            "checkedAt": 1731000000123_i64
        },
        "shareMemory": true,
        "futureKey": { "kept": [1, 2] }
    })
}

#[test]
fn profile_and_view() {
    let profile: Profile = round_trip(profile_json());
    assert_eq!(profile.auth, Some(AuthKind::Token));
    assert_eq!(profile.extra.get("futureKey"), Some(&json!({ "kept": [1, 2] })));

    let main: Profile = round_trip(json!({
        "id": "main",
        "name": "Main",
        "slug": "main",
        "color": "#d97757",
        "isDefault": true,
        "configDir": null,
        "share": { "dirs": [], "files": [], "localSettingsKeys": [] }
    }));
    assert_eq!(main.config_dir, None);

    let mut view = ProfileView::from(profile);
    let wire = serde_json::to_value(&view).unwrap();
    assert!(wire.get("tokenEnc").is_none(), "the token never reaches the renderer");
    assert_eq!(wire["futureKey"], json!({ "kept": [1, 2] }));

    view.sync = Some(SyncStatus { main_changed: vec!["CLAUDE.md".into()], local_changed: vec![] });
    let mut expected = profile_json();
    expected.as_object_mut().unwrap().remove("tokenEnc");
    expected["sync"] = json!({ "mainChanged": ["CLAUDE.md"], "localChanged": [] });
    assert_eq!(serde_json::to_value(&view).unwrap(), expected);
    let _: ProfileView = round_trip(expected);
}

#[test]
fn account_times_with_a_fraction() {
    let account: AccountInfo = serde_json::from_value(json!({ "loggedIn": false, "checkedAt": 1731000000123.75 })).unwrap();
    assert_eq!(account.checked_at, 1731000000123);
}

#[test]
fn session_with_a_turn() {
    let session: SessionInfo = round_trip(json!({
        "id": "s-1",
        "profileId": "p-work",
        "kind": "claude",
        "cwd": "C:\\code\\app",
        "title": "Fix the build",
        "status": "running",
        "startedAt": 1731000000000_i64,
        "name": "Build",
        "place": { "groupId": "g1", "afterId": "s-0" },
        "claudeSessionId": "0b8e7a52-1c1f-4f63-9d0e-3a1f6c2b9e11",
        "model": "Opus",
        "contextPct": 42,
        "turn": { "kind": "failed", "at": 1731000001000_i64, "error": "rate_limit", "text": "You've hit your limit", "limit": true, "wait": "needsEnter" },
        "moving": true,
        "arrived": { "from": "p-main", "at": 1731000000500_i64, "limit": "5-hour limit", "pickedUp": true, "auto": true }
    }));
    let turn = session.turn.as_ref().unwrap();
    assert_eq!(turn.at, 1731000001000);
    assert_eq!(
        turn.kind,
        TurnKind::Failed { error: StopError::RateLimit, text: Some("You've hit your limit".into()), limit: true, wait: Some(TurnWait::NeedsEnter) }
    );
    assert!(session.moving);
    assert!(session.arrived.as_ref().unwrap().auto);

    // `exitCode: null` and the `?: true` flags left out
    let ended: SessionInfo = serde_json::from_value(json!({
        "id": "s-2",
        "profileId": "p-work",
        "kind": "login",
        "cwd": "C:\\",
        "title": "Login",
        "status": "exited",
        "exitCode": null,
        "startedAt": 1,
        "turn": { "kind": "done", "at": 5 }
    }))
    .unwrap();
    assert_eq!(ended.exit_code, None);
    assert!(!ended.moving);
    assert_eq!(ended.turn.as_ref().unwrap().kind, TurnKind::Done { background: false });
    let wire = serde_json::to_value(&ended).unwrap();
    assert!(wire.get("moving").is_none());
    assert!(wire.get("keepAwake").is_none());
    assert!(wire.get("sleptAt").is_none());
    assert_eq!(wire["turn"], json!({ "kind": "done", "at": 5 }));

    // a sleeping tab, kept awake once it is woken
    let asleep: SessionInfo = round_trip(json!({
        "id": "s-3",
        "profileId": "p-work",
        "kind": "claude",
        "cwd": "C:\\code\\app",
        "title": "app",
        "status": "sleeping",
        "startedAt": 1,
        "keepAwake": true,
        "sleptAt": 1731000002000_i64
    }));
    assert_eq!(asleep.status, SessionStatus::Sleeping);
    assert!(asleep.keep_awake);
    assert_eq!(asleep.slept_at, Some(1731000002000));

    for turn in [
        json!({ "kind": "working", "at": 1 }),
        json!({ "kind": "done", "at": 1, "background": true }),
        json!({ "kind": "permission", "at": 1 }),
        json!({ "kind": "question", "at": 1 }),
        json!({ "kind": "plan", "at": 1 }),
        json!({ "kind": "failed", "at": 1, "error": "oauth_org_not_allowed" }),
    ] {
        let _: Turn = round_trip(turn);
    }
}

#[test]
fn turn_events_and_stop_errors() {
    let _: TurnEvent = round_trip(json!({ "kind": "needsYou", "what": "question" }));
    let _: TurnEvent = round_trip(json!({ "kind": "limitWait", "stage": "needsEnter" }));
    let _: TurnEvent = round_trip(json!({ "kind": "failure", "error": "max_output_tokens", "text": "" }));
    let _: TurnEvent = round_trip(json!({ "kind": "stop", "background": false }));
    let _: TurnEvent = round_trip(json!({ "kind": "typed" }));
    for error in STOP_ERRORS {
        assert_eq!(serde_json::to_value(error).unwrap(), json!(error.as_str()));
        assert_eq!(StopError::from_name(error.as_str()), Some(error));
    }
    assert_eq!(StopError::from_name("teapot"), None);
}

#[test]
fn layout_with_group_and_split() {
    let layout: Layout = round_trip(json!({
        "root": {
            "kind": "split",
            "id": "s3",
            "dir": "row",
            "ratio": 0.4,
            "first": { "kind": "group", "id": "g1", "tabs": ["a", "b"], "active": "b" },
            "second": {
                "kind": "split",
                "id": "s2",
                "dir": "column",
                "ratio": 0.5,
                "first": { "kind": "group", "id": "g2", "tabs": ["c"], "active": "c" },
                "second": { "kind": "group", "id": "g4", "tabs": [], "active": null }
            }
        },
        "focused": "g2",
        "seq": 4
    }));
    let Node::Split(split) = &layout.root else { panic!("a split at the root") };
    assert_eq!(split.dir, Dir::Row);
    assert!(matches!(&*split.first, Node::Group(g) if g.active.as_deref() == Some("b")));

    assert_eq!(
        serde_json::to_value(Layout::default()).unwrap(),
        json!({ "root": { "kind": "group", "id": "g1", "tabs": [], "active": null }, "focused": "g1", "seq": 1 })
    );
    let _: SessionRef = round_trip(json!({ "id": "s-9", "replaces": "s-1", "place": { "afterId": "s-3" } }));
}

#[test]
fn usage_state() {
    let usage: UsageState = round_trip(json!({
        "fiveHour": { "usedPct": 37.5, "resetsAt": 1731003600000_i64 },
        "sevenDay": { "usedPct": 12 },
        "models": [{ "name": "Fable", "usedPct": 100, "resetsAt": 1731600000000_i64 }],
        "observedAt": 1731000000000_i64
    }));
    assert_eq!(usage.seven_day.as_ref().unwrap().resets_at, None);
    assert_eq!(usage.models.as_ref().unwrap()[0].window().used_pct, 100.0);
    let _: UsageState = round_trip(json!({}));

    // usage-cache.json, with a reset time that was stored with a fraction
    let cache: UsageCache = serde_json::from_value(json!({
        "p-work": { "fiveHour": { "usedPct": 50, "resetsAt": 1731003600000.5 } },
        "main": {}
    }))
    .unwrap();
    assert_eq!(cache["p-work"].five_hour.as_ref().unwrap().resets_at, Some(1731003600000));
    assert_eq!(cache.keys().collect::<Vec<_>>(), ["main", "p-work"]);
}

#[test]
fn app_settings() {
    assert_eq!(serde_json::to_value(AppSettings::default()).unwrap(), json!({ "fontSize": 13 }));
    let settings: AppSettings = round_trip(json!({
        "claudePath": "C:\\bin\\claude.exe",
        "fontSize": 15,
        "explorerMenu": true,
        "restoreTabs": false,
        "notifications": true,
        "limitAlerts": false,
        "autoSwitch": true,
        "tray": true,
        "openAtLogin": false,
        "trayHintShown": true,
        "skillFolders": ["C:\\code\\app"],
        "someNewSetting": "kept"
    }));
    assert_eq!(settings.font_size, 15);
    assert_eq!(settings.extra.get("someNewSetting"), Some(&json!("kept")));

    // the defaults under what the file says, as `{ ...DEFAULT_SETTINGS, ...raw.settings }`
    let sparse: AppSettings = serde_json::from_value(json!({ "tray": false })).unwrap();
    assert_eq!(sparse.font_size, FONT_SIZE.default);
    assert_eq!(sparse.tray, Some(false));
    let fraction: AppSettings = serde_json::from_value(json!({ "fontSize": 13.6 })).unwrap();
    assert_eq!(fraction.font_size, 14);
}

#[test]
fn app_state_and_changed() {
    let state: AppState = round_trip(json!({
        "platform": "win32",
        "packaged": true,
        "osBuild": 26200,
        "homeDir": "C:\\Users\\me",
        "profiles": [],
        "sessions": [],
        "usage": { "main": { "observedAt": 1 } },
        "settings": { "fontSize": 13 },
        "recentFolders": ["C:\\code\\app"],
        "folderAccounts": { "c:\\code\\app": "p-work" },
        "claude": { "path": null, "error": "claude was not found" },
        "shareableDirs": ["skills", "agents"],
        "shareableFiles": ["CLAUDE.md"],
        "workflows": [],
        "runs": []
    }));
    assert_eq!(state.claude.path, None);
    let _: AppChanged = round_trip(json!({
        "settings": { "fontSize": 13 },
        "claude": { "path": "C:\\bin\\claude.exe", "version": "2.1.0" },
        "recentFolders": [],
        "folderAccounts": {}
    }));
    let _: ExplorerMenuStatus = round_trip(json!({ "supported": true, "tier": null, "available": "classic", "reason": "Not packaged" }));
    let _: Notice = round_trip(json!({ "level": "warn", "message": "Sync failed" }));
}

#[test]
fn profile_inputs_and_results() {
    let _: CreateProfileInput = round_trip(json!({
        "name": "Work",
        "color": "#5b9bd5",
        "auth": "login",
        "email": "me@example.com",
        "shareDirs": ["skills"],
        "shareFiles": [],
        "keepModelSeparate": true
    }));
    let _: SyncResult = round_trip(json!({ "needsConfirm": true, "localChanged": ["settings.json"], "linked": [], "copied": [], "backedUp": [], "warnings": [] }));
    let _: ProfilePatch = round_trip(json!({ "shareMemory": false }));
    let _: CreateSessionInput = round_trip(json!({ "profileId": "p-work", "cwd": "C:\\code", "resume": "pick", "place": {} }));
    let _: McpServerRow = round_trip(json!({ "name": "github", "transport": "http", "status": "different" }));
    let _: McpCopyResult = round_trip(json!({ "name": "github", "ok": false, "error": "Not a server of the main account" }));
}

#[test]
fn workflow_run() {
    let run: WorkflowRun = round_trip(json!({
        "id": "r-1",
        "workflowId": "w-1",
        "name": "Plan, implement, review",
        "cwd": "C:\\code\\app",
        "state": "waiting",
        "current": 1,
        "steps": [
            { "id": "a", "name": "Plan", "profileId": "p-main", "state": "done", "hasOutput": true },
            { "id": "b", "name": "Implement", "profileId": "p-work", "state": "gate", "sessionId": "s-4", "needsYou": false, "background": true, "hasOutput": true },
            { "id": "c", "name": "Review", "profileId": "p-main", "state": "paused", "hasOutput": false, "problem": "The account is not signed in." }
        ],
        "startedAt": 1731000000000_i64,
        "endedAt": 1731000900000_i64
    }));
    assert!(run.state.is_active());
    assert_eq!(run.steps[2].state, StepState::Paused);
    let _: StartRunInput = round_trip(json!({ "workflowId": "w-1", "cwd": "C:\\code", "task": "Add a test" }));
    let _: WorkflowIssue = round_trip(json!({ "stepId": "a", "level": "error", "message": "The step has no prompt." }));
    let _: PromptRef = round_trip(json!({ "kind": "step", "name": "Plan" }));
    let _: PromptRef = round_trip(json!({ "kind": "previous" }));
}

#[test]
fn workflows_file_keeps_what_it_does_not_know() {
    let file: WorkflowsFile = round_trip(json!({
        "version": 1,
        "workflows": [{
            "id": "w-1",
            "name": "Plan and build",
            "steps": [
                {
                    "id": "a",
                    "name": "Plan",
                    "profileId": "p-main",
                    "model": "fable",
                    "effort": "xhigh",
                    "permissionMode": "plan",
                    "prompt": "Plan {{task}}",
                    "gate": true,
                    "color": "teal"
                },
                {
                    "id": "b",
                    "name": "Build",
                    "profileId": "p-work",
                    "permissionMode": "acceptEdits",
                    "continues": "a",
                    "prompt": "{{previous.output}}",
                    "gate": false
                }
            ],
            "pinned": true
        }],
        "lastOpened": "w-1"
    }));
    assert_eq!(file.extra.get("lastOpened"), Some(&json!("w-1")));
    let workflow = &file.workflows[0];
    assert_eq!(workflow.extra.get("pinned"), Some(&json!(true)));
    assert_eq!(workflow.steps[0].extra.get("color"), Some(&json!("teal")));
    assert_eq!(workflow.steps[0].effort, Some(Effort::Xhigh));
    assert_eq!(workflow.steps[1].permission_mode, Some(PermissionMode::AcceptEdits));

    // a missing gate is a gate (`raw.gate !== false`); a missing version is 1; a non-list is no workflows
    let step: WorkflowStep = serde_json::from_value(json!({ "id": "x", "name": "X", "profileId": "p", "prompt": "go" })).unwrap();
    assert!(step.gate);
    let odd: WorkflowsFile = serde_json::from_value(json!({ "workflows": { "not": "a list" }, "other": 1 })).unwrap();
    assert_eq!(odd.version, 1);
    assert!(odd.workflows.is_empty());
    assert_eq!(odd.extra.get("other"), Some(&json!(1)));

    for mode in PERMISSION_MODES {
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(mode.as_str()));
    }
    for effort in EFFORT_LEVELS {
        assert_eq!(serde_json::to_value(effort).unwrap(), json!(effort.as_str()));
    }
    assert!(serde_json::from_value::<PermissionMode>(json!("bypassPermissions")).is_err());
}

#[test]
fn skills_and_their_listing() {
    let listing: SkillListing = round_trip(json!({
        "roots": [
            { "id": "p-0123456789ab", "kind": "personal", "dir": "C:\\Users\\me\\.claude\\skills", "profileIds": ["main", "p-work"], "exists": true },
            { "id": "j-0123456789ab", "kind": "project", "dir": "C:\\code\\app\\.claude\\skills", "profileIds": [], "folder": "C:\\code\\app", "exists": false, "pinned": true }
        ],
        "skills": [{
            "rootId": "p-0123456789ab",
            "dirName": "review",
            "name": "review",
            "description": "Reviews a pull request.",
            "modelInvocable": true,
            "userInvocable": true,
            "linked": false,
            "issues": [
                { "level": "warning", "scope": "claude", "message": "Same name.", "clash": true },
                { "level": "info", "scope": "upload", "message": "Long description." }
            ]
        }]
    }));
    assert_eq!(listing.skills[0].skill_ref(), SkillRef { root_id: "p-0123456789ab".into(), dir_name: "review".into() });
    assert!(listing.skills[0].issues[0].clash);
    let _: SkillRef = round_trip(json!({ "rootId": "p-0123456789ab", "dirName": "review" }));

    let detail = json!({
        "rootId": "p-0123456789ab",
        "dirName": "review",
        "text": "---\nname: review\n---\n",
        "hash": "ab",
        "files": [{ "path": "SKILL.md", "size": 22 }],
        "moreFiles": false,
        "path": "C:\\Users\\me\\.claude\\skills\\review",
        "linked": false,
        "readOnly": "The file is too large."
    });
    let saved: SaveResult = round_trip(json!({ "saved": detail }));
    assert!(matches!(saved, SaveOutcome::Saved(_)));
    let conflict: SaveResult = round_trip(json!({ "conflict": detail }));
    assert!(matches!(conflict, SaveOutcome::Conflict(_)));
    let _: SkillMoved = round_trip(json!({ "ref": { "rootId": "j-0123456789ab", "dirName": "review-2" }, "warning": "Kept a copy." }));
    let _: ImportPreview = round_trip(json!({ "token": "t1", "name": "review", "description": "", "files": [], "moreFiles": true, "issues": [] }));
    let _: FormField = round_trip(json!({
        "key": "context",
        "label": "Runs",
        "kind": "choice",
        "hint": "In a subagent ...",
        "choices": [{ "value": "", "label": "In the conversation" }, { "value": "fork", "label": "In a subagent" }],
        "accepts": ["inline"]
    }));
}

#[test]
fn agents() {
    let _: AgentListing = round_trip(json!({
        "roots": [{ "id": "p-0123456789ab", "kind": "personal", "dir": "C:\\Users\\me\\.claude\\agents", "profileIds": ["main"], "exists": true }],
        "agents": [{
            "rootId": "p-0123456789ab",
            "file": "team/qa.md",
            "name": "qa",
            "description": "Tests things.",
            "model": "sonnet",
            "effort": "",
            "loads": true,
            "issues": [{ "level": "error", "message": "No name.", "clash": true }]
        }]
    }));
    let _: AgentSaveResult = round_trip(json!({ "conflict": { "rootId": "p-0123456789ab", "file": "qa.md", "text": "", "hash": "cd", "path": "C:\\x\\qa.md" } }));
    let _: AgentMoved = round_trip(json!({ "ref": { "rootId": "p-0123456789ab", "file": "qa.md" } }));
}

#[test]
fn memory_entries_and_views() {
    let entry: MemoryEntry = round_trip(json!({
        "file": "ui-language.md",
        "title": "UI language is English",
        "description": "app text must be English",
        "type": "feedback",
        "updatedAt": 1731000000000_i64,
        "size": 412,
        "indexed": true
    }));
    assert_eq!(entry.r#type, "feedback");
    let _: MemoryView = round_trip(json!({
        "folderId": "m-0123456789ab",
        "dir": "C:\\Users\\me\\.claude\\projects\\C--code-app\\memory",
        "hasIndex": true,
        "indexLines": 3,
        "indexBytes": 280,
        "entries": [],
        "issues": [{ "level": "warning", "message": "MEMORY.md links a file that is not there." }]
    }));
    let _: MemoryListing = round_trip(json!({
        "projects": [{
            "key": "C--code-app",
            "cwd": null,
            "folders": [{ "id": "m-0123456789ab", "profileIds": ["main"], "sharedWith": ["p-work"], "dir": "C:\\m", "count": 4, "updatedAt": 1 }]
        }],
        "shareProblem": null
    }));
    let _: MemoryHit = round_trip(json!({ "folderId": "m-0123456789ab", "file": "a.md", "key": "C--code-app", "cwd": "C:\\code\\app", "profileIds": ["main"], "title": "A", "snippet": "…a…" }));
    let _: MemorySaveResult = round_trip(json!({ "saved": { "folderId": "m-0123456789ab", "file": "a.md", "text": "x", "hash": "ef", "path": "C:\\m\\a.md" } }));
    let _: AdoptResult = round_trip(json!({ "copy": [{ "from": "a.md", "to": "a.from-work.md" }], "same": ["b.md"], "target": "C:\\m", "applied": false }));
    let _: MemoryCreateInput = round_trip(json!({ "name": "n", "title": "t", "description": "d", "type": "user" }));
    for kind in MEMORY_TYPES {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.as_str()));
    }
}

#[test]
fn instructions_and_load_steps() {
    let steps: Vec<LoadStep> = round_trip(json!([
        { "path": "C:\\Users\\me\\.claude\\CLAUDE.md", "role": "Account", "when": "start", "lines": 12, "depth": 0 },
        { "path": "C:\\code\\app\\.claude\\rules\\tests.md", "role": "Rule", "when": "demand", "lines": 4, "depth": 0, "note": "Read for src/**/*.test.ts" },
        { "path": "C:\\code\\app\\docs\\gone.md", "role": "Import", "when": "missing", "lines": 0, "depth": 1 }
    ]));
    assert_eq!(steps[1].when, LoadWhen::Demand);
    let _: InstructionScope = round_trip(json!({ "id": "a-0123456789ab", "kind": "account", "profileId": "p-work", "dir": "C:\\p", "syncedClaudeMd": true }));
    let _: InstructionFile = round_trip(json!({ "file": ".claude/rules/tests.md", "kind": "rule", "exists": true, "lines": 4, "paths": ["src/**"], "imports": ["@docs/a.md"] }));
    let _: InstructionSaveResult = round_trip(json!({ "saved": { "scopeId": "f-0123456789ab", "file": "CLAUDE.md", "text": "", "hash": "00", "path": "C:\\code\\CLAUDE.md" } }));
    let _: InstructionTarget = round_trip(json!({ "rule": "tests" }));
    let _: InstructionTarget = round_trip(json!({ "file": "CLAUDE.local.md" }));
}

#[test]
fn conversation_entries() {
    let entry: ConversationEntry = round_trip(json!({
        "profileId": "p-work",
        "id": "0b8e7a52-1c1f-4f63-9d0e-3a1f6c2b9e11",
        "cwd": "C:\\code\\app",
        "title": "Fix the build",
        "firstPrompt": "The build fails",
        "updatedAt": 1731000000000_i64
    }));
    assert_eq!(entry.last_prompt, None);
}

#[test]
fn saved_tabs_file() {
    let saved: SavedTabs = round_trip(json!({
        "tabs": [
            { "profileId": "main", "cwd": "C:\\code\\app", "claudeSessionId": "0b8e7a52-1c1f-4f63-9d0e-3a1f6c2b9e11", "transcriptPath": "C:\\t.jsonl", "name": "Build" },
            { "profileId": "p-work", "cwd": "C:\\code\\lib" }
        ],
        "active": 1,
        "layout": {
            "root": { "dir": "row", "ratio": 0.5, "first": { "tabs": [0], "active": 0 }, "second": { "tabs": [1], "active": 1 } },
            "focused": 1
        }
    }));
    let layout = saved.layout.as_ref().unwrap();
    assert!(matches!(&layout.root, SavedNode::Split { dir: Dir::Row, .. }));

    // files of older versions: no layout, and a layout of the wrong shape is dropped like a missing one
    let old: SavedTabs = serde_json::from_value(json!({ "tabs": [], "active": -1 })).unwrap();
    assert_eq!(old, SavedTabs::default());
    let broken: SavedTabs = serde_json::from_value(json!({ "tabs": [], "layout": { "root": { "dir": "diagonal" } } })).unwrap();
    assert_eq!(broken.layout, None);
    assert_eq!(broken.active, -1);
    assert_eq!(serde_json::to_value(SavedTabs::default()).unwrap(), json!({ "tabs": [], "active": -1 }));

    let _: RestorePlan = round_trip(json!({
        "requests": [{ "profileId": "main", "cwd": "C:\\code", "resume": { "id": "0b8e7a52-1c1f-4f63-9d0e-3a1f6c2b9e11" }, "name": "Build" }],
        "active": 0
    }));
}

#[test]
fn config_file_root() {
    let config: AppConfig = round_trip(json!({
        "version": 1,
        "profiles": [profile_json()],
        "settings": { "fontSize": 14, "tray": false },
        "recentFolders": ["C:\\code\\app"],
        "folderAccounts": { "c:\\code\\app": "p-work" },
        "windowBounds": { "x": 10, "y": 20 }
    }));
    assert_eq!(config.profiles[0].id, "p-work");
    assert_eq!(config.extra.get("windowBounds"), Some(&json!({ "x": 10, "y": 20 })));

    // what the store did with a file that is short or odd in places
    let sparse: AppConfig = serde_json::from_value(json!({ "profiles": "nope", "settings": null, "folderAccounts": [] })).unwrap();
    assert_eq!(sparse, AppConfig::default());
    assert_eq!(
        serde_json::to_value(AppConfig::default()).unwrap(),
        json!({ "version": 1, "profiles": [], "settings": { "fontSize": 13 }, "recentFolders": [], "folderAccounts": {} })
    );
    // a broken profile is an error, never a silent loss of every account
    assert!(serde_json::from_value::<AppConfig>(json!({ "profiles": [{ "id": "x" }] })).is_err());
}

#[test]
fn ipc_payloads() {
    let _: PtyWrite = round_trip(json!({ "id": "s-1", "data": "ls\r" }));
    let resize: PtyResize = serde_json::from_value(json!({ "id": "s-1", "cols": 120, "rows": 30 })).unwrap();
    assert_eq!((resize.cols, resize.rows), (120.0, 30.0));
    let _: SessionAttention = round_trip(json!({ "id": "s-1", "title": "Done" }));
    let _: UnsavedPayload = round_trip(json!({ "items": ["The agent \"reviewer\""] }));
    assert_eq!(serde_json::from_value::<UnsavedPayload>(json!({})).unwrap(), UnsavedPayload::default());

    let _: PtyData = round_trip(json!({ "id": "s-1", "data": "\u{1b}[0m$ ", "seq": 1024 }));
    let changed: SessionsChanged = round_trip(json!({ "sessions": [] }));
    assert_eq!(changed.layout, None);
    let _: SessionsChanged = round_trip(json!({ "sessions": [], "layout": { "root": { "kind": "group", "id": "g1", "tabs": [], "active": null }, "focused": "g1", "seq": 1 } }));
    let _: OpenFolder = round_trip(json!({ "cwd": "C:\\code" }));
    let _: Activate = round_trip(json!({ "id": "s-2" }));
    assert_eq!(serde_json::to_value(NewSession {}).unwrap(), json!({}));
    let _: NewSession = round_trip(json!({}));
    let _: TransferMode = round_trip(json!("move"));

    let _: UiVisible = round_trip(json!({ "id": "s-1" }));
    assert_eq!(serde_json::from_value::<UiVisible>(json!({ "id": null })).unwrap(), UiVisible { id: None });

    assert_eq!(INVOKE_CHANNELS.len(), 93);
    assert_eq!(SEND_CHANNELS, ["pty:write", "pty:resize", "ui:layout", "session:attention", "ui:unsaved", "ui:visible"]);
    assert_eq!(EVENT_CHANNELS.len(), 13);
    let mut all: Vec<&str> = INVOKE_CHANNELS.iter().chain(&SEND_CHANNELS).chain(&EVENT_CHANNELS).copied().collect();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), 93 + 6 + 13, "every channel name is used once");
}

#[test]
fn hooks_and_the_limit_log() {
    for event in HOOK_EVENTS {
        assert_eq!(serde_json::to_value(event).unwrap(), json!(event.as_str()));
        assert_eq!(HookEvent::from_name(event.as_str()), Some(event));
    }
    let _: HookReport = round_trip(json!({ "conversationId": "0b8e7a52-1c1f-4f63-9d0e-3a1f6c2b9e11", "signal": { "kind": "stop", "text": "Done.", "background": true } }));
    let _: HookReport = round_trip(json!({ "signal": { "kind": "failure", "reason": "The account reached a usage limit.", "error": "rate_limit", "text": "" } }));
    let _: HookSignal = round_trip(json!({ "kind": "limitWait", "stage": "resumed" }));

    let log: LimitLog = round_trip(json!([
        { "at": "2026-11-08T10:00:00.000Z", "account": "Work", "event": "StopFailure", "error": "rate_limit", "text": "You've hit your limit" },
        { "at": "2026-11-08T15:00:00.000Z", "account": "Work", "event": "Notification", "type": "quota_auto_resume_fired", "text": "Resumed" }
    ]));
    assert_eq!(log[1].r#type.as_deref(), Some("quota_auto_resume_fired"));
}

#[test]
fn sessions_requests() {
    let _: SessionRequest = round_trip(json!({
        "profileId": "p-work",
        "cwd": "C:\\code",
        "kind": "claude",
        "resume": "last",
        "launch": { "model": "opus", "effort": "high", "permissionMode": "plan", "addDirs": ["C:\\req"], "watch": true },
        "prompt": "Go"
    }));
    let by_id: SessionRequest = round_trip(json!({ "profileId": "p", "cwd": "C:\\", "kind": "claude", "resume": { "id": "abc" } }));
    assert_eq!(by_id.resume, Some(SessionResume::Conversation(ResumeConversation { id: "abc".into() })));
    let _: HookNeed = round_trip(json!("none"));
    let _: HookNeed = round_trip(json!("required"));
}

#[test]
fn auto_switch_shapes() {
    let _: SwitchDecision = round_trip(json!({ "action": "move", "to": "p-2", "limit": "5-hour limit" }));
    let _: SwitchDecision = round_trip(json!({ "action": "probe", "profileId": "p-2" }));
    let _: SwitchDecision = round_trip(json!({ "action": "stay", "why": "full", "limit": "weekly limit" }));
    let none: SwitchDecision = round_trip(json!({ "action": "none", "why": "switched off" }));
    assert_eq!(none, SwitchDecision::Nothing { why: "switched off".into() });
    let _: SwitchMove = round_trip(json!({ "conversationId": "c", "from": "a", "to": "b", "at": 1 }));
    let _: UsageView = round_trip(json!({ "pct": 85, "tone": "red", "reset": false }));
    let _: UsedUp = round_trip(json!({ "label": "Fable weekly limit", "resetsAt": 5 }));
    let _: Suggestion = round_trip(json!({ "profileId": "p-2", "fiveHourPct": 12 }));
    assert_eq!(SWITCH_RULES.memory_ms, 18_000_000);
}
