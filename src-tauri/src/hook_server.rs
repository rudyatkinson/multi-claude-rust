//! Loopback HTTP server receiving Claude Code hooks (`src/main/workflows/hookServer.ts`).
//!
//! Receives the hook events of watched sessions on a local port. Each session posts to an address
//! that holds a secret of its own, so an event can be told from a stray request and belongs to
//! exactly one session.
//!
//! It is not a security boundary against programs of the same user: they can read the session's
//! settings file and post what they like. What an event can do is therefore limited. It moves a
//! workflow along, which the user approves step by step by default. And it can make the app carry
//! a conversation over to another account, but only with the setting for that switched on, when the
//! app's own reading of the account's usage agrees that a limit is reached, to an account the app
//! picks and with a message the app wrote: nothing of an event's body gets into a command line.
//!
//! Events are handed to the handler set with [`HookServer::set_handler`] one at a time, in the
//! order their requests completed, on a thread of the server's own (`hook-dispatch`): the handler
//! may block briefly (it writes `limit-events.json`, takes locks), and the answer to Claude Code
//! never waits for it.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, LazyLock, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use axum::body::{Body, HttpBody};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use axum::Router;
use mc_core::error::{AppError, AppResult};
use mc_core::hooks::{build_hook_settings, hook_url, HOOK_HOST};
use mc_core::types::{HookEvent, HOOK_EVENTS};
use regex::Regex;
use serde_json::Value;
use tauri::AppHandle;

/// A plan or a last answer can be long; more than this is not a hook event of ours.
pub const BODY_LIMIT: usize = 4 * 1024 * 1024;

static ROUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/hook/([0-9a-f]{64})/([A-Za-z]+)$").unwrap());
// JavaScript's `\b` is an ASCII word boundary
static JSON_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^application/json(?-u:\b)").unwrap());

/// What the app does with an event: `(session id, event, body)`. Called on the dispatch thread,
/// only while the secret the event came with still belongs to the session.
pub type HookHandler = Box<dyn Fn(String, HookEvent, Value) + Send + Sync>;

/// Where a registered session posts its events to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRegistration {
    /// 32 random bytes, as 64 hex digits.
    pub secret: String,
    pub port: u16,
    /// The address of every event, in [`HOOK_EVENTS`] order.
    pub urls: Vec<(HookEvent, String)>,
}

impl HookRegistration {
    fn new(port: u16, secret: String) -> Self {
        let urls = HOOK_EVENTS.iter().map(|&event| (event, hook_url(port, &secret, event))).collect();
        HookRegistration { secret, port, urls }
    }

    /// Where the session posts `event` to.
    pub fn url(&self, event: HookEvent) -> String {
        hook_url(self.port, &self.secret, event)
    }

    /// The `{ "hooks": ... }` block of the session's settings file.
    pub fn settings(&self) -> Value {
        build_hook_settings(|event| self.url(event))
    }
}

#[derive(Default)]
struct Registry {
    /// secret → session id
    session_of: HashMap<String, String>,
    /// session id → secret
    secret_of: HashMap<String, String>,
    running: Option<Running>,
}

struct Running {
    port: u16,
    /// Set by `stop`: connections that still come in are hung up on.
    stopped: Arc<AtomicBool>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

type SharedHandler = Arc<dyn Fn(String, HookEvent, Value) + Send + Sync>;

#[derive(Default)]
struct Shared {
    registry: Mutex<Registry>,
    handler: RwLock<Option<SharedHandler>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An event that was answered and waits to be handed on.
struct Dispatch {
    session_id: String,
    secret: String,
    event: HookEvent,
    body: Value,
}

impl Shared {
    /// Hands an event on, unless the session ended (or registered again) while it was on its way.
    fn deliver(&self, d: Dispatch) {
        if lock(&self.registry).secret_of.get(&d.session_id) != Some(&d.secret) {
            return;
        }
        let handler = self.handler.read().unwrap_or_else(PoisonError::into_inner).clone();
        let Some(handler) = handler else { return };
        // a hook must never bring the server down
        if catch_unwind(AssertUnwindSafe(|| handler(d.session_id, d.event, d.body))).is_err() {
            log::warn!("hook handler panicked on a {} event", d.event.as_str());
        }
    }
}

pub struct HookServer {
    shared: Arc<Shared>,
}

impl HookServer {
    /// The server, not yet listening. The app handle is not needed: what an event does is up to
    /// the handler set with [`Self::set_handler`].
    pub fn new(_app: AppHandle) -> Self {
        Self::standalone()
    }

    /// A server that needs no Tauri app (tests, tools).
    pub fn standalone() -> Self {
        HookServer { shared: Arc::default() }
    }

    /// What is done with every accepted event. Replaces the previous handler.
    pub fn set_handler(&self, handler: HookHandler) {
        *self.shared.handler.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::from(handler));
    }

    /// Starts listening on a free port of 127.0.0.1 and gives the port. Already listening, it gives
    /// the port it has. Fails when no local port can be opened.
    pub fn start(&self) -> AppResult<u16> {
        let mut registry = lock(&self.shared.registry);
        if let Some(running) = &registry.running {
            return Ok(running.port);
        }
        let failed = |e: std::io::Error| AppError::io(format!("The app could not listen for the hooks of its sessions: {e}"));
        // the literal address: "localhost" may resolve to something else
        let std_listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(failed)?;
        std_listener.set_nonblocking(true).map_err(failed)?;
        let port = std_listener.local_addr().map_err(failed)?.port();
        let runtime = tauri::async_runtime::handle();
        let listener = {
            let _entered = runtime.inner().enter();
            tokio::net::TcpListener::from_std(std_listener).map_err(failed)?
        };

        let (dispatch, queue) = mpsc::channel::<Dispatch>();
        let shared = Arc::clone(&self.shared);
        std::thread::Builder::new()
            .name("hook-dispatch".into())
            .spawn(move || {
                // ends once the server and its connections are gone
                for d in queue {
                    shared.deliver(d);
                }
            })
            .map_err(failed)?;

        let stopped = Arc::new(AtomicBool::new(false));
        let ctx = Ctx { shared: Arc::clone(&self.shared), port, dispatch };
        let router = Router::new().fallback(handle).with_state(ctx);
        let (shutdown, shutdown_signal) = tokio::sync::oneshot::channel::<()>();
        let listener = HookListener { inner: listener, stopped: Arc::clone(&stopped) };
        runtime.spawn(async move {
            let served = axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = shutdown_signal.await;
                })
                .await;
            if let Err(e) = served {
                log::warn!("hook server: {e}");
            }
        });
        registry.running = Some(Running { port, stopped, shutdown: Some(shutdown) });
        log::info!("hook server listening on {HOOK_HOST}:{port}");
        Ok(port)
    }

    /// Stops listening and forgets every session. Events already answered are not handed on.
    pub fn stop(&self) {
        let mut registry = lock(&self.shared.registry);
        if let Some(mut running) = registry.running.take() {
            running.stopped.store(true, Ordering::SeqCst);
            if let Some(shutdown) = running.shutdown.take() {
                let _ = shutdown.send(());
            }
        }
        registry.session_of.clear();
        registry.secret_of.clear();
    }

    pub fn is_listening(&self) -> bool {
        lock(&self.shared.registry).running.is_some()
    }

    /// The port while listening.
    pub fn port(&self) -> Option<u16> {
        lock(&self.shared.registry).running.as_ref().map(|r| r.port)
    }

    /// Gives a session an address of its own; an address it had before is forgotten. `None` while
    /// the server is not listening.
    pub fn register(&self, session_id: &str) -> Option<HookRegistration> {
        let mut registry = lock(&self.shared.registry);
        let port = registry.running.as_ref()?.port;
        forget(&mut registry, session_id);
        let secret = hex::encode(rand::random::<[u8; 32]>());
        registry.session_of.insert(secret.clone(), session_id.to_string());
        registry.secret_of.insert(session_id.to_string(), secret.clone());
        Some(HookRegistration::new(port, secret))
    }

    /// Forgets a session: what it posts from now on is refused.
    pub fn release(&self, session_id: &str) {
        forget(&mut lock(&self.shared.registry), session_id);
    }

    /// The `{ "hooks": ... }` block of a registered session's settings file; `None` for a session
    /// that is not registered or while the server is not listening.
    pub fn settings_block(&self, session_id: &str) -> Option<Value> {
        let registry = lock(&self.shared.registry);
        let port = registry.running.as_ref()?.port;
        let secret = registry.secret_of.get(session_id)?;
        Some(build_hook_settings(|event| hook_url(port, secret, event)))
    }
}

impl Drop for HookServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn forget(registry: &mut Registry, session_id: &str) {
    if let Some(secret) = registry.secret_of.remove(session_id) {
        registry.session_of.remove(&secret);
    }
}

// ---------- the listener ----------

/// A TCP listener that hangs up on what comes in after `stop`, so that a stopped server answers
/// nothing even before its accept loop has seen the shutdown signal.
struct HookListener {
    inner: tokio::net::TcpListener,
    stopped: Arc<AtomicBool>,
}

impl axum::serve::Listener for HookListener {
    type Io = tokio::net::TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.inner.accept().await {
                Ok((stream, addr)) => {
                    if self.stopped.load(Ordering::SeqCst) {
                        drop(stream);
                        continue;
                    }
                    return (stream, addr);
                }
                Err(e) => {
                    use std::io::ErrorKind::{ConnectionAborted, ConnectionRefused, ConnectionReset};
                    if !matches!(e.kind(), ConnectionRefused | ConnectionAborted | ConnectionReset) {
                        // e.g. out of file handles: wait for some to be closed
                        log::warn!("hook server: accept failed: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

// ---------- the request handler ----------

#[derive(Clone)]
struct Ctx {
    shared: Arc<Shared>,
    port: u16,
    dispatch: mpsc::Sender<Dispatch>,
}

fn refuse(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response.headers_mut().insert(header::CONNECTION, header::HeaderValue::from_static("close"));
    response
}

/// The secret and the event name of a hook address. The whole request target is matched, as
/// Node's `req.url`: a query string, or an absolute URI, is not an address of ours.
fn route(uri: &Uri) -> Option<(String, String)> {
    if uri.scheme().is_some() || uri.authority().is_some() {
        return None;
    }
    let target = uri.path_and_query().map_or_else(|| uri.path(), |pq| pq.as_str());
    let caps = ROUTE.captures(target)?;
    Some((caps[1].to_string(), caps[2].to_string()))
}

fn header_text(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

async fn handle(State(ctx): State<Ctx>, req: Request) -> Response {
    if req.method() != Method::POST {
        return refuse(StatusCode::METHOD_NOT_ALLOWED);
    }
    let headers = req.headers();
    // a web page that was tricked into posting here names another host or says where it comes from
    let host_ok = header_text(headers, header::HOST).is_some_and(|host| host == format!("{HOOK_HOST}:{}", ctx.port));
    if !host_ok || headers.contains_key(header::ORIGIN) {
        return refuse(StatusCode::FORBIDDEN);
    }
    if !JSON_TYPE.is_match(&header_text(headers, header::CONTENT_TYPE).unwrap_or_default()) {
        return refuse(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let Some((secret, event_name)) = route(req.uri()) else {
        return refuse(StatusCode::NOT_FOUND);
    };
    let session_id = lock(&ctx.shared.registry).session_of.get(&secret).cloned();
    let (Some(session_id), Some(event)) = (session_id, HookEvent::from_name(&event_name)) else {
        return refuse(StatusCode::NOT_FOUND);
    };
    let declared = header_text(headers, header::CONTENT_LENGTH).and_then(|n| n.trim().parse::<u64>().ok());
    if declared.is_some_and(|n| n > BODY_LIMIT as u64) {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let mut body = req.into_body();
    let mut data: Vec<u8> = Vec::new();
    while let Some(frame) = std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
        match frame {
            Ok(frame) => {
                if let Ok(chunk) = frame.into_data() {
                    if data.len() + chunk.len() > BODY_LIMIT {
                        // say why, then hang up on whatever is still being sent
                        return refuse(StatusCode::PAYLOAD_TOO_LARGE);
                    }
                    data.extend_from_slice(&chunk);
                }
            }
            // the connection broke off; nobody reads this answer
            Err(_) => return refuse(StatusCode::BAD_REQUEST),
        }
    }
    // `Buffer.toString('utf8')`: bytes that are not UTF-8 become U+FFFD instead of failing
    let Ok(body) = serde_json::from_str::<Value>(&String::from_utf8_lossy(&data)) else {
        return refuse(StatusCode::BAD_REQUEST);
    };

    // answer first: Claude waits for a hook, and what follows here may take its time
    let _ = ctx.dispatch.send(Dispatch { session_id, secret, event, body });
    let mut response = Response::new(Body::from("{}"));
    response.headers_mut().insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/json"));
    response
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    use serde_json::json;

    use super::*;

    type Received = Arc<Mutex<Vec<(String, HookEvent, Value)>>>;

    fn listening() -> (HookServer, HookRegistration, Received) {
        let server = HookServer::standalone();
        let received: Received = Arc::default();
        let sink = Arc::clone(&received);
        server.set_handler(Box::new(move |id, event, body| sink.lock().unwrap().push((id, event, body))));
        server.start().unwrap();
        let reg = server.register("tab-1").unwrap();
        (server, reg, received)
    }

    struct Sent<'a> {
        method: &'a str,
        headers: Vec<(&'a str, &'a str)>,
        body: Vec<u8>,
    }

    impl Default for Sent<'_> {
        fn default() -> Self {
            Sent { method: "POST", headers: vec![("Content-Type", "application/json")], body: br#"{"a":1}"#.to_vec() }
        }
    }

    /// Posts as Claude Code does unless told otherwise; gives the status, or 0 when the
    /// connection was cut. Adds `Host` unless a header of that name is given, as Node did.
    fn send(url: &str, sent: Sent<'_>) -> u16 {
        let rest = url.strip_prefix("http://").unwrap();
        let (authority, path) = rest.split_at(rest.find('/').unwrap());
        let Ok(mut stream) = TcpStream::connect(authority) else { return 0 };
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut head = format!("{} {path} HTTP/1.1\r\n", sent.method);
        if !sent.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            head += &format!("Host: {authority}\r\n");
        }
        for (k, v) in &sent.headers {
            head += &format!("{k}: {v}\r\n");
        }
        head += &format!("Content-Length: {}\r\nConnection: close\r\n\r\n", sent.body.len());
        let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&sent.body));
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
        let text = String::from_utf8_lossy(&response);
        text.strip_prefix("HTTP/1.1 ").and_then(|s| s.get(..3)).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    fn post(url: &str) -> u16 {
        send(url, Sent::default())
    }

    fn with_body(body: &str) -> Sent<'static> {
        Sent { body: body.as_bytes().to_vec(), ..Sent::default() }
    }

    fn with_headers<'a>(headers: Vec<(&'a str, &'a str)>) -> Sent<'a> {
        Sent { headers, ..Sent::default() }
    }

    fn settled() {
        std::thread::sleep(Duration::from_millis(60));
    }

    #[test]
    fn hands_an_event_to_its_session() {
        let (_server, reg, received) = listening();
        let stop = reg.url(HookEvent::Stop);
        assert!(Regex::new(r"^http://127\.0\.0\.1:\d+/hook/[0-9a-f]{64}/Stop$").unwrap().is_match(&stop));
        assert_eq!(send(&stop, with_body(r#"{"last_assistant_message":"done"}"#)), 200);
        settled();
        assert_eq!(*received.lock().unwrap(), vec![("tab-1".to_string(), HookEvent::Stop, json!({ "last_assistant_message": "done" }))]);
    }

    #[test]
    fn answers_with_an_empty_json_object() {
        let (_server, reg, _received) = listening();
        let url = reg.url(HookEvent::Stop);
        let rest = url.strip_prefix("http://").unwrap();
        let (authority, path) = rest.split_at(rest.find('/').unwrap());
        let mut stream = TcpStream::connect(authority).unwrap();
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.to_ascii_lowercase().contains("content-type: application/json"), "{response}");
        assert!(response.ends_with("\r\n\r\n{}"), "{response}");
    }

    #[test]
    fn gives_every_session_an_address_of_its_own() {
        let (server, reg, received) = listening();
        let other = server.register("tab-2").unwrap();
        assert_ne!(other.url(HookEvent::Stop), reg.url(HookEvent::Stop));
        assert_eq!(post(&other.url(HookEvent::UserPromptSubmit)), 200);
        settled();
        let ids: Vec<String> = received.lock().unwrap().iter().map(|r| r.0.clone()).collect();
        assert_eq!(ids, ["tab-2"]);
    }

    #[test]
    fn refuses_what_does_not_come_from_claude_code_the_way_it_posts() {
        let (_server, reg, received) = listening();
        let stop = reg.url(HookEvent::Stop);
        assert_eq!(send(&stop, Sent { method: "GET", body: Vec::new(), ..Sent::default() }), 405);
        assert_eq!(send(&stop, with_headers(vec![("Content-Type", "text/plain")])), 415);
        assert_eq!(send(&stop, with_headers(vec![])), 415);
        assert_eq!(send(&stop, with_headers(vec![("Content-Type", "application/jsonx")])), 415);
        // a page in a browser says where it comes from; a rebound DNS name shows in the host
        assert_eq!(send(&stop, with_headers(vec![("Content-Type", "application/json"), ("Origin", "https://evil.example")])), 403);
        assert_eq!(send(&stop, with_headers(vec![("Content-Type", "application/json"), ("Host", "evil.example")])), 403);
        assert_eq!(send(&stop, with_body("not json")), 400);
        // the method is looked at first, then the host, then the type
        assert_eq!(send(&stop, Sent { method: "PUT", headers: vec![("Host", "evil.example")], ..Sent::default() }), 405);
        assert_eq!(send(&stop, with_headers(vec![("Host", "evil.example")])), 403);
        settled();
        assert!(received.lock().unwrap().is_empty());
    }

    #[test]
    fn refuses_unknown_addresses_unknown_events_and_released_sessions() {
        let (server, reg, received) = listening();
        let stop = reg.url(HookEvent::Stop);
        assert_eq!(post(&stop.replace(&reg.secret, &"f".repeat(64))), 404);
        assert_eq!(post(&stop.replace("/Stop", "/SessionStart")), 404);
        assert_eq!(post(&stop.replace("/hook/", "/other/")), 404);
        assert_eq!(post(&format!("{stop}?x=1")), 404);
        assert_eq!(post(&stop.replace(&reg.secret, &reg.secret.to_uppercase())), 404);
        server.release("tab-1");
        assert_eq!(post(&stop), 404);
        settled();
        assert!(received.lock().unwrap().is_empty());
    }

    #[test]
    fn gives_a_session_that_registers_again_a_new_address_and_forgets_the_old_one() {
        let (server, reg, _received) = listening();
        let again = server.register("tab-1").unwrap();
        assert_ne!(again.secret, reg.secret);
        assert_eq!(post(&reg.url(HookEvent::Stop)), 404);
        assert_eq!(post(&again.url(HookEvent::Stop)), 200);
    }

    #[test]
    fn refuses_a_body_that_is_too_large() {
        let (_server, reg, received) = listening();
        let body = json!({ "last_assistant_message": "x".repeat(5 * 1024 * 1024) }).to_string();
        let status = send(&reg.url(HookEvent::Stop), with_body(&body));
        assert!([413, 0].contains(&status), "{status}");
        settled();
        assert!(received.lock().unwrap().is_empty());
    }

    #[test]
    fn takes_a_body_of_just_the_limit() {
        let (_server, reg, received) = listening();
        let filler = BODY_LIMIT - r#"{"last_assistant_message":""}"#.len();
        let body = json!({ "last_assistant_message": "x".repeat(filler) }).to_string();
        assert_eq!(body.len(), BODY_LIMIT);
        assert_eq!(send(&reg.url(HookEvent::Stop), with_body(&body)), 200);
        settled();
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn has_no_address_to_give_before_it_listens_or_after_it_stopped() {
        let idle = HookServer::standalone();
        assert!(!idle.is_listening());
        assert_eq!(idle.port(), None);
        assert_eq!(idle.register("tab-1"), None);
        let (server, reg, _received) = listening();
        assert!(server.is_listening());
        assert_eq!(server.port(), Some(reg.port));
        let stop = reg.url(HookEvent::Stop);
        server.stop();
        assert!(!server.is_listening());
        assert_eq!(server.register("tab-1"), None);
        assert_eq!(post(&stop), 0);
    }

    #[test]
    fn listens_again_after_it_stopped() {
        let (server, _reg, received) = listening();
        server.stop();
        let port = server.start().unwrap();
        assert_eq!(server.start().unwrap(), port);
        let reg = server.register("tab-9").unwrap();
        assert_eq!(post(&reg.url(HookEvent::Notification)), 200);
        settled();
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn keeps_serving_after_a_handler_panicked() {
        let server = HookServer::standalone();
        let received: Received = Arc::default();
        let sink = Arc::clone(&received);
        server.set_handler(Box::new(move |id, event, body| {
            if event == HookEvent::Stop {
                panic!("handler failed");
            }
            sink.lock().unwrap().push((id, event, body));
        }));
        server.start().unwrap();
        let reg = server.register("tab-1").unwrap();
        assert_eq!(post(&reg.url(HookEvent::Stop)), 200);
        assert_eq!(post(&reg.url(HookEvent::UserPromptSubmit)), 200);
        settled();
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn hands_events_on_in_the_order_they_came() {
        let (_server, reg, received) = listening();
        let order = [HookEvent::UserPromptSubmit, HookEvent::PreToolUse, HookEvent::PermissionRequest, HookEvent::Stop];
        for event in order {
            assert_eq!(post(&reg.url(event)), 200);
        }
        settled();
        let got: Vec<HookEvent> = received.lock().unwrap().iter().map(|r| r.1).collect();
        assert_eq!(got, order);
    }

    #[test]
    fn does_not_hand_on_what_a_released_session_posted() {
        let server = HookServer::standalone();
        let received: Received = Arc::default();
        let sink = Arc::clone(&received);
        server.set_handler(Box::new(move |id, event, body| sink.lock().unwrap().push((id, event, body))));
        server.start().unwrap();
        let reg = server.register("tab-1").unwrap();
        let answered = || Dispatch { session_id: "tab-1".into(), secret: reg.secret.clone(), event: HookEvent::Stop, body: json!({}) };
        server.shared.deliver(answered());
        assert_eq!(received.lock().unwrap().len(), 1);
        // the session ended while the event was on its way
        server.release("tab-1");
        server.shared.deliver(answered());
        // or it registered again: the old secret is no longer its own
        server.register("tab-1").unwrap();
        server.shared.deliver(answered());
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[test]
    fn writes_the_settings_block_of_a_registered_session() {
        let (server, reg, _received) = listening();
        let block = server.settings_block("tab-1").unwrap();
        assert_eq!(block, reg.settings());
        assert_eq!(block["hooks"]["Stop"][0]["hooks"][0]["url"], json!(reg.url(HookEvent::Stop)));
        assert_eq!(block["hooks"]["PreToolUse"][0]["matcher"], json!("ExitPlanMode"));
        assert_eq!(reg.urls.len(), HOOK_EVENTS.len());
        assert_eq!(server.settings_block("tab-2"), None);
        server.stop();
        assert_eq!(server.settings_block("tab-1"), None);
    }
}
