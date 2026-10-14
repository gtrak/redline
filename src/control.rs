//! Control socket (plan 019 issue 02): a unix-domain NDJSON server that lets
//! an external client drive the LIVE TUI — it wraps the very same
//! `Arc<Mutex<AppStore>>` the render loop owns, so a remote command mutates
//! the session a human is watching.
//!
//! The wire format is one JSON `Request` per line and one JSON `Response` per
//! line (`\n`-delimited). The command vocabulary mirrors the headless CLI
//! (`src/session.rs`): `ping`, `state`, `quote`, `buffer`, `notes`, `do`,
//! `key`, `keys`, `text`, `quit`.
//!
//! After EVERY request the server bumps the store's `repaint_bus` so the
//! render loop repaints from the already-mutated store.
#![allow(dead_code)] // the client half has no non-test caller in some builds

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::app::store::AppStore;
use crate::session::{Session, SessionError};

/// One client request line.
#[derive(Deserialize, Serialize, Debug)]
pub struct Request {
    pub id: u64,
    pub cmd: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

/// One server response line.
#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

/// A wire error: a stable `code` plus a human `message`.
#[derive(Serialize, Deserialize, Debug)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

fn wire_err(code: &str, message: impl Into<String>) -> WireError {
    WireError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// Resolve the control socket path. Precedence (documented, deterministic):
/// 1. the explicit `--socket` argument,
/// 2. `$REDLINE_SOCKET`,
/// 3. `$XDG_RUNTIME_DIR/redline-<uid>.sock`,
/// 4. `/tmp/redline-<uid>.sock` (the system temp dir).
///
/// `<uid>` is `$UID` when set, else the process id. No `libc` dependency:
/// `getuid` is unavailable and `/proc/self` metadata is fiddly, so `$UID` (set
/// by every login shell) with a pid fallback is the pragmatic deterministic
/// choice.
pub fn socket_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Ok(p) = std::env::var("REDLINE_SOCKET")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    let uid = uid_token();
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join(format!("redline-{uid}.sock"));
    }
    std::env::temp_dir().join(format!("redline-{uid}.sock"))
}

fn uid_token() -> String {
    std::env::var("UID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::process::id().to_string())
}

/// Serve the control socket until the process exits.
///
/// A stale socket file (a previous process crashed without unlinking) is
/// removed before binding — unless something is still listening on it, in
/// which case binding fails with a clear error.
pub async fn serve(sock: PathBuf, store: Arc<Mutex<AppStore>>) -> anyhow::Result<()> {
    if sock.exists() {
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            anyhow::bail!("control socket already in use: {}", sock.display());
        }
        let _ = std::fs::remove_file(&sock);
    }
    let listener = UnixListener::bind(&sock)
        .map_err(|e| anyhow::anyhow!("bind {}: {e}", sock.display()))?;
    tracing::info!(path = %sock.display(), "control socket listening");
    loop {
        let (stream, _addr) = listener.accept().await?;
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, store).await {
                tracing::warn!("control connection error: {e}");
            }
        });
    }
}

async fn handle_connection(stream: UnixStream, store: Arc<Mutex<AppStore>>) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(line) {
            Ok(req) => apply_request(&store, &req),
            Err(e) => Response {
                id: 0,
                ok: false,
                result: None,
                error: Some(wire_err("bad-request", e.to_string())),
            },
        };
        let mut out = serde_json::to_vec(&response)?;
        out.push(b'\n');
        write_half.write_all(&out).await?;
        write_half.flush().await?;
    }
    Ok(())
}

/// Apply one request against the live store and build its response.
///
/// The store lock is NEVER held across an await: [`dispatch`] locks, mutates,
/// and drops the guard before returning; the caller writes the response after.
/// After every request the repaint bus is bumped exactly once.
pub fn apply_request(store: &Arc<Mutex<AppStore>>, req: &Request) -> Response {
    // Derive the session root/geometry from the live store (the socket's
    // `Session` wraps the SAME `Arc<Mutex<AppStore>>` the TUI uses).
    let (root, width, height) = {
        let s = store.lock().unwrap();
        let root = s
            .project
            .as_ref()
            .map(|p| p.root.clone())
            .unwrap_or_default();
        (root, 80u16, 24u16)
    };
    let mut session = Session::from_store(Arc::clone(store), root, width, height);
    let outcome = dispatch(&mut session, req);
    store.lock().unwrap().repaint_bus().bump();
    match outcome {
        Ok(value) => Response {
            id: req.id,
            ok: true,
            result: Some(value),
            error: None,
        },
        Err(err) => Response {
            id: req.id,
            ok: false,
            result: None,
            error: Some(err),
        },
    }
}

fn map_session_err(err: SessionError) -> WireError {
    match err {
        SessionError::UnknownCommand(name) => wire_err("unknown-command", format!("unknown command `{name}`")),
        SessionError::BadKey(token) => wire_err("bad-key", format!("bad key token: {token}")),
    }
}

fn dispatch(session: &mut Session, req: &Request) -> Result<serde_json::Value, WireError> {
    let arg_str = |key: &str| -> Option<String> {
        req.args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    };
    match req.cmd.as_str() {
        "ping" => Ok(serde_json::json!({})),
        "state" => serde_json::to_value(session.snapshot())
            .map_err(|e| wire_err("internal", e.to_string())),
        "quote" => Ok(serde_json::json!({ "text": session.quote() })),
        "buffer" => {
            let start = req.args.get("start").and_then(|v| v.as_u64());
            let end = req.args.get("end").and_then(|v| v.as_u64());
            let range = match (start, end) {
                (Some(s), Some(e)) => Some((s as usize, e as usize)),
                _ => None,
            };
            Ok(serde_json::json!({ "text": session.buffer_text(range) }))
        }
        "notes" => {
            let plain = req
                .args
                .get("plain")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(serde_json::json!({ "text": session.notes(plain) }))
        }
        "do" => {
            let name = arg_str("name")
                .ok_or_else(|| wire_err("bad-request", "`do` needs a `name`"))?;
            let arg = arg_str("arg");
            session.do_command(&name, arg).map_err(map_session_err)?;
            Ok(serde_json::json!({}))
        }
        "key" => {
            let token =
                arg_str("token").ok_or_else(|| wire_err("bad-request", "`key` needs a `token`"))?;
            session.key(&token).map_err(map_session_err)?;
            Ok(serde_json::json!({}))
        }
        "keys" => {
            let seq = arg_str("seq").ok_or_else(|| wire_err("bad-request", "`keys` needs a `seq`"))?;
            session.keys(&seq).map_err(map_session_err)?;
            Ok(serde_json::json!({}))
        }
        "text" => {
            let text =
                arg_str("text").ok_or_else(|| wire_err("bad-request", "`text` needs `text`"))?;
            session.text(&text).map_err(map_session_err)?;
            Ok(serde_json::json!({}))
        }
        "quit" => {
            // Route through the registry `quit` (`begin_quit`): it prompts on
            // modified buffers, then flips `store.quit`, which Root's
            // `if snap.quit { system.exit() }` turns into an exit.
            let _ = session.do_command("quit", None);
            Ok(serde_json::json!({}))
        }
        other => Err(wire_err(
            "unknown-command",
            format!("unknown control command `{other}`"),
        )),
    }
}

/// The blocking std client (no tokio needed client-side).
pub mod client {
    use super::{Request, Response};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    /// Connect to the control socket (used to probe liveness before sending).
    pub fn try_connect(sock: &Path) -> std::io::Result<UnixStream> {
        UnixStream::connect(sock)
    }

    /// Send one request and read one response line. `Ok(None)` when the
    /// server closed the connection without a reply.
    pub fn send(sock: &Path, req: &Request) -> anyhow::Result<Response> {
        let stream = UnixStream::connect(sock)?;
        send_on(stream, req)
    }

    /// Send one request over an already-connected stream.
    pub fn send_on(mut stream: UnixStream, req: &Request) -> anyhow::Result<Response> {
        let mut line = serde_json::to_vec(req)?;
        line.push(b'\n');
        stream.write_all(&line)?;
        stream.flush()?;
        let mut reader = BufReader::new(stream);
        let mut resp = String::new();
        reader.read_line(&mut resp)?;
        let trimmed = resp.trim();
        if trimmed.is_empty() {
            anyhow::bail!("empty response from control socket");
        }
        Ok(serde_json::from_str(trimmed)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::store::AppStore;
    use std::sync::Mutex;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"probe\"\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# probe\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "// hello\n").unwrap();
        dir
    }

    fn live_store(dir: &tempfile::TempDir) -> Arc<Mutex<AppStore>> {
        Arc::new(Mutex::new(AppStore::at(dir.path(), dir.path().join("state"))))
    }

    fn req(id: u64, cmd: &str, args: serde_json::Value) -> Request {
        Request {
            id,
            cmd: cmd.to_string(),
            args,
        }
    }

    #[test]
    fn apply_request_round_trips() {
        let dir = project();
        let store = live_store(&dir);
        // `watch::Sender::send` does NOT store the value when no receiver is
        // alive (documented tokio semantics), so keep a keeper receiver for
        // the duration of the test — the real UI holds one via the repaint
        // drain.
        let _keeper = store.lock().unwrap().repaint_bus().subscribe();

        let ping = apply_request(&store, &req(1, "ping", serde_json::json!({})));
        assert!(ping.ok, "ping ok: {ping:?}");

        let state = apply_request(&store, &req(2, "state", serde_json::json!({})));
        assert!(state.ok, "state ok: {state:?}");
        assert!(
            state.result.as_ref().unwrap().get("view").is_some(),
            "state carries a view: {state:?}"
        );

        // Open Cargo.toml via the picker the way a human does.
        let open = apply_request(
            &store,
            &req(3, "do", serde_json::json!({"name": "find-file"})),
        );
        assert!(open.ok, "do find-file ok: {open:?}");
        let text = apply_request(&store, &req(4, "text", serde_json::json!({"text": "Cargo.toml"})));
        assert!(text.ok, "text ok: {text:?}");
        let ret = apply_request(&store, &req(5, "key", serde_json::json!({"token": "RET"})));
        assert!(ret.ok, "key RET ok: {ret:?}");

        let quote = apply_request(&store, &req(6, "quote", serde_json::json!({})));
        assert!(quote.ok, "quote ok: {quote:?}");
        let frame = quote.result.as_ref().unwrap()["text"].as_str().unwrap();
        assert!(frame.contains("Cargo.toml"), "frame names the opened file: {frame:?}");

        // Unknown command maps to the stable error code.
        let unknown = apply_request(&store, &req(7, "bogus", serde_json::json!({})));
        assert!(!unknown.ok);
        assert_eq!(unknown.error.as_ref().unwrap().code, "unknown-command");

        // The repaint bus advanced for every request (the keeper receiver is
        // still alive, so the stored count reflects the bumps).
        let n = *store.lock().unwrap().repaint_bus().subscribe().borrow();
        assert!(n >= 7, "repaint bus bumped per request, got {n}");
    }

    #[test]
    fn socket_path_prefers_explicit_then_env_then_default() {
        let _guard = crate::ENV_LOCK.lock().unwrap();
        let explicit = PathBuf::from("/tmp/explicit.sock");
        assert_eq!(socket_path(Some(&explicit)), explicit);

        // Snapshot + restore the env we touch so the test is hermetic.
        let prior_socket = std::env::var("REDLINE_SOCKET").ok();
        let prior_runtime = std::env::var("XDG_RUNTIME_DIR").ok();
        // SAFETY: `set_var`/`remove_var` race with concurrent env reads on
        // other threads; the `crate::ENV_LOCK` guard above spans this whole
        // test body, serializing env mutation against every other
        // env-mutating/reading test in the crate.
        unsafe {
            std::env::set_var("REDLINE_SOCKET", "/tmp/from-env.sock");
        }
        assert_eq!(
            socket_path(None),
            PathBuf::from("/tmp/from-env.sock"),
            "REDLINE_SOCKET beats XDG/default"
        );

        // SAFETY: still under `crate::ENV_LOCK` (as above).
        unsafe {
            std::env::remove_var("REDLINE_SOCKET");
        }
        // SAFETY: still under `crate::ENV_LOCK` (as above).
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/9999");
        }
        let p = socket_path(None);
        assert_eq!(p.parent(), Some(Path::new("/run/user/9999")));
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("redline-"));

        match prior_socket {
            // SAFETY: still under `crate::ENV_LOCK` (as above).
            Some(v) => unsafe { std::env::set_var("REDLINE_SOCKET", v) },
            // SAFETY: still under `crate::ENV_LOCK` (as above).
            None => unsafe { std::env::remove_var("REDLINE_SOCKET") },
        }
        match prior_runtime {
            // SAFETY: still under `crate::ENV_LOCK` (as above).
            Some(v) => unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) },
            // SAFETY: still under `crate::ENV_LOCK` (as above).
            None => unsafe { std::env::remove_var("XDG_RUNTIME_DIR") },
        }
    }

    // A multi-thread runtime: the client half uses BLOCKING std unix-socket
    // IO, which would starve a current-thread runtime's spawned `serve` task
    // (the test thread would block the only worker).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serve_round_trip() {
        let dir = project();
        let store = live_store(&dir);
        let sock = dir.path().join("ctl.sock");

        let serve_store = Arc::clone(&store);
        let serve_sock = sock.clone();
        let handle = tokio::spawn(async move { serve(serve_sock, serve_store).await });

        // Wait for the listener to bind (connect retry).
        let mut connected = None;
        for _ in 0..100 {
            match client::try_connect(&sock) {
                Ok(s) => {
                    connected = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        }
        let stream = connected.expect("control socket should accept connections");

        let ping = client::send_on(stream, &req(1, "ping", serde_json::json!({})))
            .expect("ping round-trip");
        assert!(ping.ok, "ping ok: {ping:?}");

        let open = client::send(&sock, &req(2, "do", serde_json::json!({"name": "open-palette"})))
            .expect("do round-trip");
        assert!(open.ok, "do open-palette ok: {open:?}");

        handle.abort();
        let _ = std::fs::remove_file(&sock);
    }
}
