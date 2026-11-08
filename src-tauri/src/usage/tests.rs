use std::sync::Mutex;
use std::time::Instant;

use mc_core::types::{ModelUsage, UsageWindow};
use mc_core::types::NoticeLevel;

use super::prober::ProbeRun;
use super::*;

const HOUR: i64 = 3_600_000;

struct Deps(Vec<ProbeProfile>);

impl ProbeDeps for Deps {
    fn claude_path(&self) -> Option<PathBuf> {
        None
    }
    fn profiles(&self) -> Vec<ProbeProfile> {
        self.0.clone()
    }
    fn env_for(&self, _profile: &ProbeProfile) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }
}

fn account(id: &str, name: &str) -> ProbeProfile {
    ProbeProfile { id: id.into(), name: name.into(), config_dir: None, logged_in: true, token: false }
}

struct Setup {
    _root: tempfile::TempDir,
    tracker: UsageTracker,
    emitted: Arc<Mutex<Vec<UsageCache>>>,
}

fn never_run() -> ProbeRun {
    Arc::new(|_, _, _| Box::pin(async { Err("no command in tests".to_string()) }))
}

fn setup_in(root: tempfile::TempDir) -> Setup {
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let sink = emitted.clone();
    let tracker = UsageTracker::build(root.path().to_path_buf(), Box::new(move |all: &UsageCache| sink.lock().unwrap().push(all.clone())), never_run());
    fs::create_dir_all(root.path().join("run")).unwrap();
    Setup { _root: root, tracker, emitted }
}

fn setup() -> Setup {
    setup_in(tempfile::tempdir().unwrap())
}

fn status_json(session: &str, pct: f64, resets_at_s: i64) -> String {
    format!(
        r#"{{"session_id":"{session}","model":{{"display_name":"Opus"}},"context_window":{{"used_percentage":12.6}},"rate_limits":{{"five_hour":{{"used_percentage":{pct},"resets_at":{resets_at_s}}}}}}}"#
    )
}

/// A reset time a few hours ahead, in epoch seconds, so that the window has not reset.
fn later_s() -> i64 {
    (now_ms() + 3 * HOUR) / 1000
}

#[test]
fn ingests_a_status_file_and_reports_the_session() {
    let s = setup();
    let metas = Arc::new(Mutex::new(Vec::new()));
    let sink = metas.clone();
    s.tracker.set_session_meta_sink(move |session, meta| sink.lock().unwrap().push((session.to_string(), meta)));
    let reset = later_s();
    let files = s.tracker.files_for("p1", "s1");
    assert!(files.status_file.ends_with("p1.s1.status.json"));
    fs::write(&files.status_file, status_json("claude-1", 23.5, reset)).unwrap();

    s.tracker.core.scan();
    let usage = s.tracker.usage_of("p1").unwrap();
    assert_eq!(usage.five_hour, Some(UsageWindow { used_pct: 23.5, resets_at: Some(reset * 1000) }));
    assert_eq!(s.emitted.lock().unwrap().len(), 1);
    let metas = metas.lock().unwrap();
    assert_eq!(metas[0].0, "s1");
    assert_eq!(metas[0].1.claude_session_id.as_deref(), Some("claude-1"));
    assert_eq!(metas[0].1.context_pct, Some(13));
    drop(metas);

    // an unchanged file is not read again by the scan, and the same numbers change nothing
    s.tracker.core.scan();
    s.tracker.core.ingest("p1.s1.status.json");
    assert_eq!(s.emitted.lock().unwrap().len(), 1);
}

#[test]
fn session_ended_takes_the_last_sample_and_removes_the_files() {
    let s = setup();
    let files = s.tracker.files_for("p1", "s1");
    fs::write(&files.settings_file, "{}").unwrap();
    fs::write(&files.status_file, status_json("c", 61.0, later_s())).unwrap();
    let tmp = files.status_file.with_file_name("p1.s1.status.json.tmp");
    fs::write(&tmp, "{").unwrap();

    s.tracker.session_ended("p1", "s1");
    assert_eq!(s.tracker.usage_of("p1").and_then(|u| u.five_hour).map(|w| w.used_pct), Some(61.0));
    assert!(!files.settings_file.exists());
    assert!(!files.status_file.exists());
    assert!(!tmp.exists());
}

#[test]
fn saves_the_cache_on_stop_and_reads_it_back() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().to_path_buf();
    let s = setup_in(root);
    let result = ProbeResult {
        five_hour: Some(UsageWindow { used_pct: 40.0, resets_at: Some(now_ms() + HOUR) }),
        seven_day: None,
        models: vec![ModelUsage { name: "Fable".into(), used_pct: 35.0, resets_at: None }],
    };
    s.tracker.apply_probe("p1", &result);
    s.tracker.stop();
    let saved = cache::load(&path.join("usage-cache.json"));
    assert_eq!(saved, s.tracker.snapshot());
    assert_eq!(saved["p1"].models.as_ref().map(Vec::len), Some(1));

    let again = UsageTracker::build(path.clone(), Box::new(|_: &UsageCache| {}), never_run());
    assert_eq!(again.snapshot(), saved);
    again.prune(["p2"]);
    assert!(again.snapshot().is_empty());
    assert!(cache::load(&path.join("usage-cache.json")).is_empty());
}

#[test]
fn alerts_go_to_the_notifier_with_the_account_name() {
    let s = setup();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let sink = messages.clone();
    s.tracker.set_notifier(move |profile, message| sink.lock().unwrap().push((profile.to_string(), message)));
    s.tracker.set_deps(Arc::new(Deps(vec![account("p1", "Work")])));
    let window = |pct: f64| ProbeResult { five_hour: Some(UsageWindow { used_pct: pct, resets_at: Some(now_ms() + HOUR) }), ..Default::default() };

    // the first numbers are a baseline
    s.tracker.apply_probe("p1", &window(50.0));
    s.tracker.apply_probe("p1", &window(85.0));
    // nobody to name: nothing is said
    s.tracker.apply_probe("ghost", &window(50.0));
    s.tracker.apply_probe("ghost", &window(99.0));

    let messages = messages.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].0, "p1");
    assert_eq!(messages[0].1.title, "Work: 5-hour limit at 85%");
    assert!(messages[0].1.body.starts_with("Resets in "), "{}", messages[0].1.body);
    assert_eq!(messages[0].1.level, NoticeLevel::Warn);
}

#[test]
fn purges_old_files_at_start_and_watches_for_new_ones() {
    let s = setup();
    let run = s.tracker.core.run_dir.clone();
    let old = run.join("old.x.status.json");
    fs::write(&old, "{}").unwrap();
    let file = fs::File::options().write(true).open(&old).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(8 * 24 * 3600)).unwrap();
    drop(file);
    let fresh = run.join("p1.s9.settings.json");
    fs::write(&fresh, "{}").unwrap();

    s.tracker.start();
    assert!(!old.exists());
    assert!(fresh.exists());

    // written the way the tap writes it: a temporary file renamed over the status file
    let tmp = run.join("p2.s1.status.json.tmp");
    fs::write(&tmp, status_json("c", 7.0, later_s())).unwrap();
    fs::rename(&tmp, run.join("p2.s1.status.json")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.tracker.usage_of("p2").is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(s.tracker.usage_of("p2").and_then(|u| u.five_hour).map(|w| w.used_pct), Some(7.0));
    s.tracker.stop();
}

#[test]
fn builds_the_status_line_setting_of_a_session() {
    let s = setup();
    let project = s._root.path().join("project");
    let config = s._root.path().join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("settings.json"), r#"{"statusLine":{"type":"command","command":"my-line","padding":1}}"#).unwrap();
    let setting = s.tracker.status_line_setting(&project, &config, "p1", "s1", "Work");
    if !s.tracker.tap_supported() {
        assert_eq!(setting, None);
        return;
    }
    let line = &setting.unwrap()["statusLine"];
    assert_eq!(line["padding"], 1);
    let command = line["command"].as_str().unwrap();
    let status = tap::posix_path(&s.tracker.files_for("p1", "s1").status_file.to_string_lossy());
    assert!(command.contains(&format!("'{status}'")), "{command}");
    assert!(command.ends_with("| { my-line\n}"));
}

#[test]
fn refresh_without_deps_answers_false() {
    let s = setup();
    assert!(!async_runtime::block_on(s.tracker.refresh("p1")));
    s.tracker.stop();
    assert!(!async_runtime::block_on(s.tracker.refresh("p1")));
}
