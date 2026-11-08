//! Hook settings block and `parse_hook` (`src/main/workflows/hooks.ts`), and the entries of
//! `limit-events.json` (`limitEventOf` of `src/main/limitLog.ts`).
//!
//! What a session reports through Claude Code's hooks, and how to read it. The events are sent by
//! `http` hooks in the session's settings file. Claude Code merges them with the hooks of the user
//! and of the project; it does not send `SessionStart` over http.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Map, Value};

use crate::jsstr::cut16;
use crate::turn::{js_trim, stop_sentence};
use crate::types::{
    AskWhat, HookEvent, HookReport, HookSignal, LimitEvent, LimitWaitStage, StopError, HOOK_EVENTS, LIMIT_LOG_TEXT,
    OUTPUT_LIMIT,
};

/// The literal loopback address the hook server listens on: "localhost" may resolve to something else.
pub const HOOK_HOST: &str = "127.0.0.1";
/// A hook that got no answer delays Claude by this long at most, in seconds.
pub const HOOK_TIMEOUT_S: u64 = 5;
/// As much of an error's text as is kept: enough to tell what it was.
pub const ERROR_TEXT_LIMIT: usize = 300;

static CONVERSATION_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap());

/// Where a session posts an event to: `http://127.0.0.1:<port>/hook/<secret>/<Event>`.
pub fn hook_url(port: u16, secret: &str, event: HookEvent) -> String {
    format!("http://{HOOK_HOST}:{port}/hook/{secret}/{}", event.as_str())
}

/// The `hooks` part of a session's settings file, as `{ "hooks": { ... } }`. `url_for` gives the
/// address an event is posted to.
///
/// The keys come out in alphabetical order (serde_json's map), not in [`HOOK_EVENTS`] order as
/// the TypeScript wrote them; Claude Code does not care about the order of keys.
pub fn build_hook_settings(url_for: impl Fn(HookEvent) -> String) -> Value {
    let mut hooks = Map::new();
    for event in HOOK_EVENTS {
        let handler = json!({ "type": "http", "url": url_for(event), "timeout": HOOK_TIMEOUT_S });
        // of the tools only the one that presents a plan matters; permission requests cover the rest
        let entry = if event == HookEvent::PreToolUse {
            json!({ "matcher": "ExitPlanMode", "hooks": [handler] })
        } else {
            json!({ "hooks": [handler] })
        };
        hooks.insert(event.as_str().to_string(), Value::Array(vec![entry]));
    }
    json!({ "hooks": hooks })
}

/// `str(value)`: the string, or `""` for anything else.
fn str_of(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

/// The first of the fields that holds a non-empty string, or `""`.
fn first_str<'a>(body: &'a Map<String, Value>, keys: &[&str]) -> &'a str {
    keys.iter().map(|k| str_of(body.get(*k))).find(|s| !s.is_empty()).unwrap_or("")
}

fn failure(body: &Map<String, Value>) -> HookSignal {
    let kind = first_str(body, &["error", "error_type", "reason"]);
    let error = StopError::from_name(kind).unwrap_or(StopError::Unknown);
    let text = cut16(first_str(body, &["last_assistant_message", "error_details", "message"]), ERROR_TEXT_LIMIT);
    // an error this version has no name for is at least told by the name it came with
    let told = if !text.is_empty() {
        text
    } else if error == StopError::Unknown {
        kind
    } else {
        ""
    };
    HookSignal::Failure { reason: stop_sentence(error, told), error, text: text.to_string() }
}

// a permission prompt, or a form or a link of an MCP server, that has waited a few seconds
const NEEDS_AN_ANSWER: [&str; 3] = ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog"];

fn limit_wait(kind: &str) -> Option<LimitWaitStage> {
    match kind {
        "quota_auto_resume_fired" => Some(LimitWaitStage::Resumed),
        "quota_auto_resume_stale" => Some(LimitWaitStage::NeedsEnter),
        "quota_auto_resume_disabled" => Some(LimitWaitStage::Ended),
        _ => None,
    }
}

/// What a posted hook event says about its session. `None` for events that say nothing worth
/// knowing and for bodies that are not what Claude Code sends (not a JSON object).
pub fn parse_hook(event: HookEvent, body: &Value) -> Option<HookReport> {
    let body = body.as_object()?;
    let id = str_of(body.get("session_id"));
    let conversation_id = CONVERSATION_ID.is_match(id).then(|| id.to_string());
    let report = |signal: HookSignal| Some(HookReport { conversation_id: conversation_id.clone(), signal });
    let tool = str_of(body.get("tool_name"));
    let plan = || {
        let input = str_of(body.get("tool_input").and_then(Value::as_object).and_then(|input| input.get("plan")));
        if js_trim(input).is_empty() {
            return None;
        }
        report(HookSignal::Plan { text: cut16(input, OUTPUT_LIMIT).to_string() })
    };

    match event {
        HookEvent::UserPromptSubmit => report(HookSignal::Prompt),
        HookEvent::Stop => report(HookSignal::Stop {
            text: cut16(str_of(body.get("last_assistant_message")), OUTPUT_LIMIT).to_string(),
            background: body.get("background_tasks").and_then(Value::as_array).is_some_and(|tasks| !tasks.is_empty()),
        }),
        HookEvent::StopFailure => report(failure(body)),
        HookEvent::PreToolUse => {
            if tool == "ExitPlanMode" {
                plan()
            } else {
                None
            }
        }
        HookEvent::PermissionRequest => {
            if tool == "ExitPlanMode" {
                return plan();
            }
            let what = if tool == "AskUserQuestion" { AskWhat::Question } else { AskWhat::Permission };
            report(HookSignal::NeedsYou { what })
        }
        HookEvent::Notification => {
            let kind = str_of(body.get("notification_type"));
            if let Some(stage) = limit_wait(kind) {
                return report(HookSignal::LimitWait { stage });
            }
            if kind == "agent_needs_input" {
                return report(HookSignal::NeedsYou { what: AskWhat::Question });
            }
            // an idle prompt only says that Claude is done, which `Stop` said already
            if NEEDS_AN_ANSWER.contains(&kind) {
                return report(HookSignal::NeedsYou { what: AskWhat::Permission });
            }
            None
        }
    }
}

// ---------- limit-events.json ----------

/// `head`: a non-empty string, cut to [`LIMIT_LOG_TEXT`] units.
fn head(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(cut16(s, LIMIT_LOG_TEXT).to_string()),
        _ => None,
    }
}

/// The entry of `limit-events.json` for a hook event: a failed turn, or the end of Claude Code's
/// wait for a usage limit. `None` for events that are neither of the two. `account` is the name of
/// the profile, `now` the time in ms since the epoch.
pub fn limit_event_of(event: HookEvent, body: &Value, account: &str, now: i64) -> Option<LimitEvent> {
    // `typeof body === 'object'`: an array passes too, as an object without fields
    let empty = Map::new();
    let sent = match body {
        Value::Object(map) => map,
        Value::Array(_) => &empty,
        _ => return None,
    };
    let base = LimitEvent {
        at: iso_string(now),
        account: account.to_string(),
        event,
        error: None,
        details: None,
        r#type: None,
        text: None,
    };
    if event == HookEvent::StopFailure {
        return Some(LimitEvent {
            error: head(sent.get("error")).or_else(|| head(sent.get("error_type"))).or_else(|| head(sent.get("reason"))),
            details: head(sent.get("error_details")),
            text: head(sent.get("last_assistant_message")),
            ..base
        });
    }
    let kind = head(sent.get("notification_type"));
    match kind {
        Some(kind) if event == HookEvent::Notification && kind.starts_with("quota_") => {
            Some(LimitEvent { r#type: Some(kind), text: head(sent.get("message")), ..base })
        }
        _ => None,
    }
}

/// `new Date(ms).toISOString()`: `2026-10-08T12:00:00.000Z`, with the six-digit signed year of
/// JavaScript outside 0000-9999.
pub fn iso_string(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    let (h, m, s, milli) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    let year = if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    };
    format!("{year}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{milli:03}Z")
}

/// Days since 1970-01-01 to (year, month, day) of the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsstr::len16;
    use crate::types::STOP_ERRORS;

    static PAYLOADS: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../../../src-tauri/tests/fixtures/hook-payloads.json")).unwrap()
    });

    fn body(name: &str) -> &'static Value {
        &PAYLOADS[name]["body"]
    }

    fn signal(event: HookEvent, body: Value) -> Option<HookSignal> {
        parse_hook(event, &body).map(|r| r.signal)
    }

    fn failure_of(body: Value) -> (String, StopError, String) {
        match signal(HookEvent::StopFailure, body) {
            Some(HookSignal::Failure { reason, error, text }) => (reason, error, text),
            other => panic!("not a failure: {other:?}"),
        }
    }

    // ---------- buildHookSettings ----------

    #[test]
    fn posts_every_event_to_its_own_address_and_looks_at_one_tool_only() {
        let settings = build_hook_settings(|event| format!("http://127.0.0.1:1/hook/x/{}", event.as_str()));
        let hooks = settings["hooks"].as_object().unwrap();
        let mut keys: Vec<&str> = hooks.keys().map(String::as_str).collect();
        let mut expected: Vec<&str> = HOOK_EVENTS.iter().map(|e| e.as_str()).collect();
        keys.sort_unstable();
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(settings.as_object().unwrap().len(), 1);
        assert_eq!(hooks["Stop"], json!([{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:1/hook/x/Stop", "timeout": 5 }] }]));
        assert_eq!(
            hooks["PreToolUse"],
            json!([{ "matcher": "ExitPlanMode", "hooks": [{ "type": "http", "url": "http://127.0.0.1:1/hook/x/PreToolUse", "timeout": 5 }] }])
        );
    }

    #[test]
    fn hook_urls() {
        let secret = "ab".repeat(32);
        assert_eq!(hook_url(4242, &secret, HookEvent::StopFailure), format!("http://127.0.0.1:4242/hook/{secret}/StopFailure"));
    }

    // ---------- parseHook, with what Claude Code really sends ----------

    #[test]
    fn reads_the_start_and_the_end_of_a_turn() {
        let conversation = body("stop")["session_id"].as_str().unwrap().to_string();
        assert_eq!(
            parse_hook(HookEvent::UserPromptSubmit, body("prompt")),
            Some(HookReport { conversation_id: Some(conversation.clone()), signal: HookSignal::Prompt })
        );
        assert_eq!(
            parse_hook(HookEvent::Stop, body("stop")),
            Some(HookReport {
                conversation_id: Some(conversation),
                signal: HookSignal::Stop { text: "pong".into(), background: false }
            })
        );
    }

    #[test]
    fn reads_a_presented_plan_from_either_event_that_carries_it() {
        let plan = body("planPreToolUse")["tool_input"]["plan"].as_str().unwrap().to_string();
        assert!(plan.contains("PLAN-MARKER-7731"));
        assert_eq!(parse_hook(HookEvent::PreToolUse, body("planPreToolUse")).unwrap().signal, HookSignal::Plan { text: plan.clone() });
        assert_eq!(parse_hook(HookEvent::PermissionRequest, body("planPermission")).unwrap().signal, HookSignal::Plan { text: plan });
    }

    #[test]
    fn reads_a_permission_request_a_question_and_the_notification_that_follows() {
        let permission = HookSignal::NeedsYou { what: AskWhat::Permission };
        assert_eq!(parse_hook(HookEvent::PermissionRequest, body("writePermission")).unwrap().signal, permission);
        assert_eq!(
            parse_hook(HookEvent::PermissionRequest, body("question")).unwrap().signal,
            HookSignal::NeedsYou { what: AskWhat::Question }
        );
        assert_eq!(parse_hook(HookEvent::Notification, body("notification")).unwrap().signal, permission);
    }

    #[test]
    fn the_fixture_names_its_events() {
        for name in ["prompt", "stop", "planPreToolUse", "planPermission", "writePermission", "question", "notification"] {
            let event = HookEvent::from_name(PAYLOADS[name]["event"].as_str().unwrap()).unwrap();
            assert!(parse_hook(event, body(name)).is_some(), "{name}");
        }
    }

    // ---------- parseHook ----------

    #[test]
    fn notes_background_work_that_will_wake_claude_again() {
        assert_eq!(
            signal(HookEvent::Stop, json!({ "last_assistant_message": "started", "background_tasks": [{ "id": "b1" }] })),
            Some(HookSignal::Stop { text: "started".into(), background: true })
        );
    }

    #[test]
    fn gives_an_answer_of_any_shape_a_text_and_cuts_one_that_is_too_long() {
        let empty = Some(HookSignal::Stop { text: String::new(), background: false });
        assert_eq!(signal(HookEvent::Stop, json!({})), empty);
        assert_eq!(signal(HookEvent::Stop, json!({ "last_assistant_message": 42 })), empty);
        let long = signal(HookEvent::Stop, json!({ "last_assistant_message": "x".repeat(OUTPUT_LIMIT + 10) }));
        match long {
            Some(HookSignal::Stop { text, .. }) => assert_eq!(len16(&text), OUTPUT_LIMIT),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn names_the_usual_api_errors_and_keeps_what_they_were() {
        assert_eq!(
            failure_of(json!({ "error": "rate_limit" })),
            ("The account reached a usage limit.".into(), StopError::RateLimit, String::new())
        );
        assert_eq!(
            failure_of(json!({ "error_type": "authentication_failed" })),
            ("The account is no longer signed in.".into(), StopError::AuthenticationFailed, String::new())
        );
        assert_eq!(
            failure_of(json!({ "error": "odd_one", "error_details": "Something broke" })),
            ("Claude stopped with an error: Something broke".into(), StopError::Unknown, "Something broke".into())
        );
        assert_eq!(
            failure_of(json!({ "error": "odd_one" })),
            ("Claude stopped with an error: odd_one".into(), StopError::Unknown, String::new())
        );
        assert_eq!(failure_of(json!({})), ("Claude stopped with an error.".into(), StopError::Unknown, String::new()));
        // the reason is taken when the other two are missing or empty
        assert_eq!(failure_of(json!({ "error": "", "reason": "overloaded" })).1, StopError::Overloaded);
    }

    #[test]
    fn has_a_sentence_of_its_own_for_every_error_claude_code_names() {
        let reasons: Vec<String> = STOP_ERRORS
            .into_iter()
            .filter(|e| *e != StopError::Unknown)
            .map(|error| {
                let (reason, got, _) = failure_of(json!({ "error": error.as_str() }));
                assert_eq!(got, error);
                reason
            })
            .collect();
        let distinct: std::collections::HashSet<&String> = reasons.iter().collect();
        assert_eq!(distinct.len(), STOP_ERRORS.len() - 1);
        assert!(reasons.iter().all(|r| !r.is_empty() && !r.starts_with("Claude stopped")));
    }

    // as the documentation gives it: no such event has been caught from a real session yet
    #[test]
    fn keeps_the_text_claude_code_showed_for_an_error_the_beginning_of_a_long_one() {
        let documented = json!({ "error": "rate_limit", "error_details": "429 Too Many Requests", "last_assistant_message": "API Error: Rate limit reached" });
        let (_, error, text) = failure_of(documented);
        assert_eq!((error, text.as_str()), (StopError::RateLimit, "API Error: Rate limit reached"));
        assert_eq!(failure_of(json!({ "error": "server_error", "error_details": "500" })).2, "500");
        let (_, _, long) = failure_of(json!({ "error": "unknown", "last_assistant_message": "x".repeat(5000) }));
        assert_eq!(len16(&long), 300);
        // the text of a notification-like body comes last
        assert_eq!(failure_of(json!({ "message": "said so" })).2, "said so");
    }

    #[test]
    fn tells_a_throttling_server_from_a_limit_of_the_account() {
        let (reason, error, _) = failure_of(
            json!({ "error": "rate_limit", "last_assistant_message": "API Error: Server is temporarily limiting requests (not your usage limit)" }),
        );
        assert_eq!((error, reason.as_str()), (StopError::RateLimit, "The Claude servers are limiting requests for the moment."));
        let (reason, error, _) = failure_of(json!({ "error": "unknown", "last_assistant_message": "You've hit your session limit · resets 3:45pm" }));
        assert_eq!((error, reason.as_str()), (StopError::Unknown, "The account reached a usage limit."));
    }

    #[test]
    fn reads_how_the_wait_of_claude_code_for_a_limit_ended() {
        let stage = |kind: &str| signal(HookEvent::Notification, json!({ "notification_type": kind }));
        assert_eq!(stage("quota_auto_resume_fired"), Some(HookSignal::LimitWait { stage: LimitWaitStage::Resumed }));
        assert_eq!(stage("quota_auto_resume_stale"), Some(HookSignal::LimitWait { stage: LimitWaitStage::NeedsEnter }));
        assert_eq!(stage("quota_auto_resume_disabled"), Some(HookSignal::LimitWait { stage: LimitWaitStage::Ended }));
    }

    #[test]
    fn reads_the_other_notifications_that_wait_for_an_answer() {
        let of = |kind: &str| signal(HookEvent::Notification, json!({ "notification_type": kind }));
        assert_eq!(of("elicitation_dialog"), Some(HookSignal::NeedsYou { what: AskWhat::Permission }));
        assert_eq!(of("elicitation_url_dialog"), Some(HookSignal::NeedsYou { what: AskWhat::Permission }));
        assert_eq!(of("agent_needs_input"), Some(HookSignal::NeedsYou { what: AskWhat::Question }));
    }

    #[test]
    fn ignores_what_means_nothing_to_a_step() {
        assert_eq!(parse_hook(HookEvent::PreToolUse, &json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } })), None);
        assert_eq!(parse_hook(HookEvent::PreToolUse, &json!({ "tool_name": "ExitPlanMode", "tool_input": { "plan": "  " } })), None);
        assert_eq!(parse_hook(HookEvent::PreToolUse, &json!({ "tool_name": "ExitPlanMode" })), None);
        assert_eq!(parse_hook(HookEvent::PreToolUse, &json!({ "tool_name": "ExitPlanMode", "tool_input": ["plan"] })), None);
        // a blank plan in a permission request says nothing either
        assert_eq!(parse_hook(HookEvent::PermissionRequest, &json!({ "tool_name": "ExitPlanMode", "tool_input": { "plan": "\u{FEFF}\n" } })), None);
        assert_eq!(parse_hook(HookEvent::Notification, &json!({ "notification_type": "idle_prompt" })), None);
        assert_eq!(parse_hook(HookEvent::Notification, &json!({ "notification_type": "auth_success" })), None);
        assert_eq!(parse_hook(HookEvent::Notification, &json!({ "notification_type": "toString" })), None);
        assert_eq!(parse_hook(HookEvent::Stop, &json!("text")), None);
        assert_eq!(parse_hook(HookEvent::Stop, &Value::Null), None);
        assert_eq!(parse_hook(HookEvent::Stop, &json!([])), None);
    }

    #[test]
    fn takes_a_conversation_id_only_when_it_is_one() {
        assert_eq!(parse_hook(HookEvent::UserPromptSubmit, &json!({ "session_id": "../../etc" })).unwrap().conversation_id, None);
        assert_eq!(parse_hook(HookEvent::UserPromptSubmit, &json!({ "session_id": 5 })).unwrap().conversation_id, None);
        let upper = "FEB42E5D-528E-4833-871B-2022EA08BA0E";
        assert_eq!(
            parse_hook(HookEvent::UserPromptSubmit, &json!({ "session_id": upper })).unwrap().conversation_id.as_deref(),
            Some(upper)
        );
        assert_eq!(
            parse_hook(HookEvent::UserPromptSubmit, &json!({ "session_id": format!("{upper}\n") })).unwrap().conversation_id,
            None
        );
    }

    #[test]
    fn a_report_serializes_as_the_typescript_did() {
        let report = parse_hook(HookEvent::UserPromptSubmit, &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&report).unwrap(), json!({ "signal": { "kind": "prompt" } }));
    }

    // ---------- limitEventOf (test/turn.test.ts) ----------

    #[test]
    fn notes_a_failed_turn_as_it_was_sent_also_with_a_name_it_does_not_know() {
        let body = json!({ "error": "brand_new", "error_details": "418", "last_assistant_message": "API Error: teapot", "session_id": "x" });
        // Date.UTC(2026, 9, 8, 12)
        let entry = limit_event_of(HookEvent::StopFailure, &body, "Main", 1_791_460_800_000).unwrap();
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            json!({
                "at": "2026-10-08T12:00:00.000Z",
                "account": "Main",
                "event": "StopFailure",
                "error": "brand_new",
                "details": "418",
                "text": "API Error: teapot"
            })
        );
        let long = limit_event_of(HookEvent::StopFailure, &json!({ "last_assistant_message": "x".repeat(900) }), "Main", 0).unwrap();
        assert_eq!(long.text.map(|t| len16(&t)), Some(300));
        // `error_type`, then `reason`, stand in for a missing name
        let typed = limit_event_of(HookEvent::StopFailure, &json!({ "error": "", "reason": "why" }), "Main", 0).unwrap();
        assert_eq!(typed.error.as_deref(), Some("why"));
    }

    #[test]
    fn notes_the_end_of_a_wait_for_a_limit_and_nothing_else() {
        let entry = limit_event_of(
            HookEvent::Notification,
            &json!({ "notification_type": "quota_auto_resume_stale", "message": "Your usage limit has reset" }),
            "Work",
            0,
        )
        .unwrap();
        assert_eq!(entry.account, "Work");
        assert_eq!(entry.r#type.as_deref(), Some("quota_auto_resume_stale"));
        assert_eq!(entry.text.as_deref(), Some("Your usage limit has reset"));
        assert_eq!(entry.at, "1970-01-01T00:00:00.000Z");
        assert_eq!(limit_event_of(HookEvent::Notification, &json!({ "notification_type": "permission_prompt" }), "Work", 0), None);
        assert_eq!(limit_event_of(HookEvent::Stop, &json!({ "last_assistant_message": "done" }), "Work", 0), None);
        assert_eq!(limit_event_of(HookEvent::StopFailure, &Value::Null, "Work", 0), None);
        assert_eq!(limit_event_of(HookEvent::StopFailure, &json!("text"), "Work", 0), None);
        // an array is an object to `typeof`
        let listed = limit_event_of(HookEvent::StopFailure, &json!([]), "Work", 0).unwrap();
        assert_eq!(listed.error, None);
    }

    #[test]
    fn iso_strings() {
        assert_eq!(iso_string(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_string(1_791_460_800_000), "2026-10-08T12:00:00.000Z");
        assert_eq!(iso_string(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(iso_string(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(iso_string(253_402_300_800_000), "+010000-01-01T00:00:00.000Z");
        assert_eq!(iso_string(-62_198_755_200_000), "-000001-01-01T00:00:00.000Z");
    }
}
