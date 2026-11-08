//! `foldText` and `searchConversations` of `src/shared/conversations.ts`.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use crate::types::ConversationEntry;
use crate::usage::is_js_space;

/// The dotless i is a letter of its own, not an i with a mark, so it is the one letter that taking
/// the marks off does not reach.
const DOTLESS_I: char = '\u{131}';

/// Text as it is compared: lower case and without diacritics, so that "isik" finds "Işık" and
/// "cagri" finds "Çağrı", as people type when the keyboard layout is not at hand.
pub fn fold_text(text: &str) -> String {
    let bare: String = text.nfd().filter(|&c| !is_combining_mark(c)).collect();
    bare.to_lowercase().replace(DOTLESS_I, "i")
}

/// The conversations in which every word of the query occurs: in the title, the first or the last
/// message, the folder or the account's name (`account_names`: profile id to name). Newest first.
/// An empty query matches all.
pub fn search_conversations<'a>(
    entries: &'a [ConversationEntry],
    query: &str,
    account_names: &BTreeMap<String, String>,
) -> Vec<&'a ConversationEntry> {
    let folded = fold_text(query);
    let words: Vec<&str> = folded.split(is_js_space).filter(|w| !w.is_empty()).collect();
    let mut matches: Vec<&ConversationEntry> = entries
        .iter()
        .filter(|entry| {
            if words.is_empty() {
                return true;
            }
            let fields = [
                Some(entry.title.as_str()),
                entry.first_prompt.as_deref(),
                entry.last_prompt.as_deref(),
                Some(entry.cwd.as_str()),
                account_names.get(&entry.profile_id).map(String::as_str),
            ];
            let haystack = fold_text(&fields.into_iter().flatten().filter(|f| !f.is_empty()).collect::<Vec<_>>().join("\n"));
            words.iter().all(|word| haystack.contains(word))
        })
        .collect();
    // stable, as `Array.prototype.sort` is
    matches.sort_by_key(|entry| Reverse(entry.updated_at));
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- searchConversations (test/conversations.test.ts) ----------

    fn entry(profile_id: &str, id: &str, cwd: &str, title: &str, first: Option<&str>, last: Option<&str>, updated_at: i64) -> ConversationEntry {
        ConversationEntry {
            profile_id: profile_id.into(),
            id: id.into(),
            cwd: cwd.into(),
            title: title.into(),
            first_prompt: first.map(str::to_string),
            last_prompt: last.map(str::to_string),
            updated_at,
        }
    }

    fn ids(query: &str) -> Vec<String> {
        let entries = [
            entry("main", "a", r"C:\code\shop", "Fix login redirect", Some("the login page loops"), None, 100),
            entry("elif", "b", r"C:\code\shop", "Işık ayarları", Some("IŞIK değerlerini düzelt"), None, 300),
            entry("main", "c", r"D:\games\İstanbul", "Profiler report", None, Some("check the login cost"), 200),
        ];
        let names = BTreeMap::from([("main".to_string(), "Bilal".to_string()), ("elif".to_string(), "Elif".to_string())]);
        search_conversations(&entries, query, &names).into_iter().map(|e| e.id.clone()).collect()
    }

    #[test]
    fn lists_everything_newest_first_for_an_empty_query() {
        assert_eq!(ids(""), ["b", "c", "a"]);
        assert_eq!(ids("   "), ["b", "c", "a"]);
    }

    #[test]
    fn needs_every_word_in_the_title_the_messages_the_folder_or_the_account() {
        assert_eq!(ids("login"), ["c", "a"]);
        assert_eq!(ids("login shop"), ["a"]);
        assert_eq!(ids("elif shop"), ["b"]);
        assert_eq!(ids("bilal"), ["c", "a"]);
        assert!(ids("nothing here").is_empty());
    }

    #[test]
    fn finds_turkish_words_however_they_are_typed() {
        assert_eq!(ids("isik"), ["b"]);
        assert_eq!(ids("IŞIK"), ["b"]);
        assert_eq!(ids("ışık"), ["b"]);
        assert_eq!(ids("degerlerini duzelt"), ["b"]);
        assert_eq!(ids("istanbul"), ["c"]);
        assert_eq!(ids("İSTANBUL"), ["c"]);
        assert_eq!(fold_text("İSTANBUL Iğdır ışık Çağrı Öğün"), "istanbul igdir isik cagri ogun");
    }
}
