//! `usage-cache.json`: the last known usage of each account, kept across restarts.

use std::fs;
use std::io;
use std::path::Path;

use mc_core::types::{UsageCache, UsageState};
use serde_json::Value;

/// The cache as it was saved. A missing or broken file is an empty cache, and an entry that does
/// not read as usage is dropped; the cache is only a convenience.
pub(crate) fn load(file: &Path) -> UsageCache {
    let Ok(text) = fs::read_to_string(file) else { return UsageCache::new() };
    let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(&text) else { return UsageCache::new() };
    entries.into_iter().filter_map(|(id, value)| serde_json::from_value::<UsageState>(value).ok().map(|state| (id, state))).collect()
}

/// `writeJsonAtomic`: pretty JSON with a final newline, through a temporary file and a rename.
pub(crate) fn save(file: &Path, cache: &UsageCache) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(cache).map_err(io::Error::other)?;
    text.push('\n');
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    fs::write(&tmp, text)?;
    // Windows can refuse the rename while another process has the target open; retry once
    fs::rename(&tmp, file).or_else(|_| fs::rename(&tmp, file)).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use mc_core::types::UsageWindow;

    use super::*;

    #[test]
    fn round_trips_and_tolerates_a_broken_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usage-cache.json");
        assert!(load(&file).is_empty());
        let cache = UsageCache::from([(
            "p1".to_string(),
            UsageState { five_hour: Some(UsageWindow { used_pct: 12.0, resets_at: Some(5) }), observed_at: Some(9), ..Default::default() },
        )]);
        save(&file, &cache).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "{\n  \"p1\": {\n    \"fiveHour\": {\n      \"usedPct\": 12,\n      \"resetsAt\": 5\n    },\n    \"observedAt\": 9\n  }\n}\n");
        assert_eq!(load(&file), cache);

        fs::write(&file, r#"{"p1":{"fiveHour":{"usedPct":3}},"p2":7}"#).unwrap();
        assert_eq!(load(&file).keys().collect::<Vec<_>>(), ["p1"]);
        fs::write(&file, "[]").unwrap();
        assert!(load(&file).is_empty());
    }
}
