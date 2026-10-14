//! redline — a read-focused TUI code browser (plan 001, issue 01).
//!
//! Entry point: load config, initialize tracing to a log file, start
//! the iocraft fullscreen render loop on the tokio runtime, and restore
//! the terminal cleanly on every exit path including panics.
//!
//! Module layout: `app/` is plain Rust (store, registry, keymap,
//! config) with zero iocraft/tokio dependencies; the headless data
//! models (project, files, buffers, the magit section tree) live in
//! the `redline-model` workspace crate (plan 014 stage 3); `ui/` holds
//! all iocraft components; this file only starts/stops the render loop.

mod app;
mod control;
mod index_profile;
mod nav;
mod search;
mod session;
mod theme;
mod ui;

#[cfg(test)]
mod gate_registry;
#[cfg(test)]
mod perf;
#[cfg(test)]
pub(crate) mod test_support;

/// The ONE lock every test that mutates process-global environment
/// holds while its env guard is live (P3b): `std::env::set_var`
/// mutates process state, and other threads reading env vars
/// concurrently is UB per the Rust docs (any variable — a reader of
/// `COLORTERM` races a writer of `HOME`) — so env-mutating tests
/// serialize on this shared test-util mutex (separate per-module
/// locks would not exclude each other). The definition moved to
/// `redline-testutil` (plan 014 stage 2) when `redline_git`'s
/// `commit.rs` `EnvScope` tests left the bin: re-exported here so
/// every `crate::ENV_LOCK` call site keeps compiling. Users:
/// `redline_git` `commit.rs`'s `EnvScope` (+ its tests' per-body
/// `TZ`/`GIT_AUTHOR_*` pins),
/// `redline_model::files`'s `EnvGuard` (+ `redline_model::project`'s usage), and the
/// `ui::file_view` tint tests' `COLORTERM` pins. Known residual (P1
/// finding, issue-guardrails): the `app::store` fetch tests read/mutate
/// Note: the fetch tests mutate `PATH` under `crate::ENV_LOCK`. They used to hold a local
/// `PATH_LOCK`, which could not exclude other environment readers — and since the environment
/// block is process-global, a narrower lock made the crate-wide invariant look protected when it
/// was not. The recommendation below is therefore already done.
/// this lock (issue-git-test-harness lane owns that file).
#[cfg(test)]
pub(crate) use redline_testutil::ENV_LOCK;

use app::config;
use app::store::AppStore;
use iocraft::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use ui::root::Root;

/// CLI args. `--notes=plain` switches the quit-dump shape (plan 005
/// issue 03); `--index-profile[=PATH]` runs the headless index profiler
/// (`src/index_profile.rs`) and exits before any terminal/TUI setup.
/// Issue 019-01b adds the headless subcommand mode: a positional
/// subcommand (`state`/`quote`/`buffer`/`notes`/`do`/`key`/`keys`/`text`/
/// `quit`) plus repeatable `-c CMD` lines, `--project`, `--json`,
/// `--width`, `--height`, and the parsed-but-unused `--socket`.
/// Unknown args are a HARD ERROR (clear feedback beats silently
/// ignoring them): usage on stderr, exit 2 — before the TUI starts.
#[derive(Debug, PartialEq, Eq)]
struct CliArgs {
    /// `--notes=plain`: the quit-dump prints in the grep/pipe shape.
    notes_plain: bool,
    /// `--index-profile[=PATH]`: run the headless index profiler (None =
    /// the normal TUI).
    index_profile: Option<PathBuf>,
    /// `--profile-top=N` (default 25): top-N rows in the report.
    profile_top: usize,
    /// `--profile-out=FILE`: write the per-file CSV.
    profile_out: Option<PathBuf>,
    /// `--profile-repeat=N` (default 1): repeat the run (warm page cache).
    profile_repeat: usize,
    /// `--profile-real-paths`: print real paths (default = anonymized).
    profile_real_paths: bool,
    /// The headless subcommand (first non-flag arg), if any.
    subcommand: Option<Subcommand>,
    /// Positional args AFTER the subcommand (`do <name> [arg]`, `buffer
    /// START..END`, `text <literal>`, ...). Joined with single spaces when a
    /// subcommand expects one string argument.
    sub_args: Vec<String>,
    /// `--project DIR` (default `.`): the project root a headless session
    /// opens.
    project: PathBuf,
    /// `--json`: emit the snapshot as JSON (only meaningful for `state`).
    json: bool,
    /// `--width N` (default 80): the headless session / render width.
    width: u16,
    /// `--height N` (default 24): the headless session / render height.
    height: u16,
    /// Repeatable `-c CMD` command lines (raw; split at run time).
    commands: Vec<String>,
    /// `--socket PATH`: the control socket to bind (TUI) or connect to
    /// (headless client). 019-02.
    socket: Option<PathBuf>,
    /// `--attached` (W15, 019-03): require a reachable control socket — a
    /// missing socket is a hard error (exit 1) even for an env-only socket,
    /// instead of the headless fallback.
    attached: bool,
}

/// A headless subcommand name (issue 019-01b; sugar verbs issue 019-03).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Subcommand {
    State,
    Quote,
    Buffer,
    Notes,
    Do,
    Key,
    Keys,
    Text,
    Quit,
    // ── sugar verbs (019-03) ──
    Open,
    Goto,
    Jump,
    Search,
    Isearch,
    Select,
    Confirm,
    Cancel,
    Annotate,
    Magit,
}

impl Subcommand {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "state" => Some(Subcommand::State),
            "quote" => Some(Subcommand::Quote),
            "buffer" => Some(Subcommand::Buffer),
            "notes" => Some(Subcommand::Notes),
            "do" => Some(Subcommand::Do),
            "key" => Some(Subcommand::Key),
            "keys" => Some(Subcommand::Keys),
            "text" => Some(Subcommand::Text),
            "quit" => Some(Subcommand::Quit),
            "open" => Some(Subcommand::Open),
            "goto" => Some(Subcommand::Goto),
            "jump" => Some(Subcommand::Jump),
            "search" => Some(Subcommand::Search),
            "isearch" => Some(Subcommand::Isearch),
            "select" => Some(Subcommand::Select),
            "confirm" => Some(Subcommand::Confirm),
            "cancel" => Some(Subcommand::Cancel),
            "annotate" => Some(Subcommand::Annotate),
            "magit" => Some(Subcommand::Magit),
            _ => None,
        }
    }

    /// The canonical verb name (usage/error text).
    fn name(self) -> &'static str {
        match self {
            Subcommand::State => "state",
            Subcommand::Quote => "quote",
            Subcommand::Buffer => "buffer",
            Subcommand::Notes => "notes",
            Subcommand::Do => "do",
            Subcommand::Key => "key",
            Subcommand::Keys => "keys",
            Subcommand::Text => "text",
            Subcommand::Quit => "quit",
            Subcommand::Open => "open",
            Subcommand::Goto => "goto",
            Subcommand::Jump => "jump",
            Subcommand::Search => "search",
            Subcommand::Isearch => "isearch",
            Subcommand::Select => "select",
            Subcommand::Confirm => "confirm",
            Subcommand::Cancel => "cancel",
            Subcommand::Annotate => "annotate",
            Subcommand::Magit => "magit",
        }
    }

    /// Whether the verb refuses any argument. W10/W11: a trailing token on
    /// one of these is a usage error (exit 2), never silently ignored.
    fn takes_no_arg(self) -> bool {
        matches!(
            self,
            Subcommand::State
                | Subcommand::Quote
                | Subcommand::Notes
                | Subcommand::Cancel
                | Subcommand::Confirm
                | Subcommand::Magit
                | Subcommand::Quit
        )
    }

    /// Whether the verb REQUIRES an argument (the sugar verbs whose whole
    /// point is the argument). `select` is optional (defaults to `next`).
    fn requires_arg(self) -> bool {
        matches!(
            self,
            Subcommand::Open
                | Subcommand::Goto
                | Subcommand::Jump
                | Subcommand::Search
                | Subcommand::Isearch
                | Subcommand::Annotate
        )
    }

    /// Whether this is a sugar verb (routed through `Session::verb`).
    fn is_sugar(self) -> bool {
        matches!(
            self,
            Subcommand::Open
                | Subcommand::Goto
                | Subcommand::Jump
                | Subcommand::Search
                | Subcommand::Isearch
                | Subcommand::Select
                | Subcommand::Confirm
                | Subcommand::Cancel
                | Subcommand::Annotate
                | Subcommand::Magit
        )
    }
}

/// Validate a verb's trailing text (W10/W11). `rest` is the raw argument
/// string after the verb (from a `-c` line or the joined positionals).
fn validate_verb_args(sub: Subcommand, rest: &str) -> Result<(), String> {
    let rest = rest.trim();
    if sub.takes_no_arg() && !rest.is_empty() {
        return Err(format!(
            "`{}` takes no arguments (got `{}`)",
            sub.name(),
            rest
        ));
    }
    if sub.requires_arg() && rest.is_empty() {
        return Err(format!("`{}` needs an argument", sub.name()));
    }
    Ok(())
}

impl CliArgs {
    const DEFAULT_TOP: usize = 25;
    const DEFAULT_WIDTH: u16 = 80;
    const DEFAULT_HEIGHT: u16 = 24;
}

fn usage() {
    eprintln!("usage: redline [--notes=plain]");
    eprintln!("       redline --index-profile[=PATH] [--profile-top=N --profile-out=FILE --profile-repeat=N --profile-real-paths]");
    eprintln!("       redline [--project DIR] [--json] [--width N] [--height N] [--socket PATH] [--attached] [-c CMD]... [SUBCOMMAND [ARGS]]");
    eprintln!("       primitives: state | quote | buffer [START..END] | notes | do NAME [ARG] | key TOKEN | keys SEQ | text LITERAL | quit");
    eprintln!("       sugar: open PATH[:LINE[:COL]] | goto LINE | jump SYMBOL | search QUERY | isearch QUERY | select [N|next|prev] | confirm | cancel | annotate TEXT | magit");
}

/// Split a `-c` line on the FIRST run of whitespace into `verb` + `rest`
/// (the 019-01b grammar: no `:` splitting). The verb must be a known
/// subcommand, and the trailing text must satisfy [`validate_verb_args`]
/// (W10/W11: a verb that takes no argument rejects stray tokens).
fn split_cmd_line(raw: &str) -> Result<(Subcommand, String), String> {
    let raw = raw.trim();
    let (verb, rest) = match raw.split_once(char::is_whitespace) {
        Some((v, r)) => (v, r.trim()),
        None => (raw, ""),
    };
    let sub = Subcommand::parse(verb)
        .ok_or_else(|| format!("unknown subcommand in -c: `{verb}`"))?;
    validate_verb_args(sub, rest)?;
    Ok((sub, rest.to_string()))
}

/// Split a `do` argument into the registry command name and its optional
/// argument (first whitespace run separates them).
fn split_do(rest: &str) -> (String, Option<String>) {
    let rest = rest.trim();
    match rest.split_once(char::is_whitespace) {
        Some((name, arg)) => {
            let arg = arg.trim();
            (
                name.to_string(),
                if arg.is_empty() {
                    None
                } else {
                    Some(arg.to_string())
                },
            )
        }
        None => (rest.to_string(), None),
    }
}

/// Parse an optional `START..END` (1-based inclusive) for `buffer`.
fn parse_range(rest: &str) -> Option<(usize, usize)> {
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let (a, b) = rest.split_once("..")?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Parse `args` (without the program name) into [`CliArgs`]. Errors are
/// the user-facing message (the caller prints usage + exits 2).
fn parse_cli(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut cli = CliArgs {
        notes_plain: false,
        index_profile: None,
        profile_top: CliArgs::DEFAULT_TOP,
        profile_out: None,
        profile_repeat: 1,
        profile_real_paths: false,
        subcommand: None,
        sub_args: Vec::new(),
        project: PathBuf::from("."),
        json: false,
        width: CliArgs::DEFAULT_WIDTH,
        height: CliArgs::DEFAULT_HEIGHT,
        commands: Vec::new(),
        socket: None,
        attached: false,
    };
    // `--profile-top` / `--profile-repeat` validate to a positive int.
    let positive = |rest: &str, flag: &str| -> Result<usize, String> {
        rest.parse::<usize>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| format!("{flag} expects a positive integer, got `{rest}`"))
    };
    let args: Vec<String> = args.into_iter().collect();
    let mut positionals: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        // A flag that consumes the NEXT arg (the value form).
        let take_value = |flag: &str, i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        if arg == "--notes=plain" {
            cli.notes_plain = true;
        } else if arg == "--json" {
            cli.json = true;
        } else if arg == "--project" {
            cli.project = PathBuf::from(take_value("--project", &mut i)?);
        } else if let Some(p) = arg.strip_prefix("--project=") {
            if p.is_empty() {
                return Err("--project= requires a path".into());
            }
            cli.project = PathBuf::from(p);
        } else if arg == "--width" {
            let v = take_value("--width", &mut i)?;
            cli.width = v
                .parse::<u16>()
                .ok()
                .filter(|w| *w > 0)
                .ok_or_else(|| format!("--width expects a positive integer, got `{v}`"))?;
        } else if let Some(v) = arg.strip_prefix("--width=") {
            cli.width = v
                .parse::<u16>()
                .ok()
                .filter(|w| *w > 0)
                .ok_or_else(|| format!("--width expects a positive integer, got `{v}`"))?;
        } else if arg == "--height" {
            let v = take_value("--height", &mut i)?;
            cli.height = v
                .parse::<u16>()
                .ok()
                .filter(|h| *h > 0)
                .ok_or_else(|| format!("--height expects a positive integer, got `{v}`"))?;
        } else if let Some(v) = arg.strip_prefix("--height=") {
            cli.height = v
                .parse::<u16>()
                .ok()
                .filter(|h| *h > 0)
                .ok_or_else(|| format!("--height expects a positive integer, got `{v}`"))?;
        } else if arg == "-c" {
            cli.commands.push(take_value("-c", &mut i)?);
        } else if arg == "--socket" {
            cli.socket = Some(PathBuf::from(take_value("--socket", &mut i)?));
        } else if let Some(p) = arg.strip_prefix("--socket=") {
            cli.socket = Some(PathBuf::from(p));
        } else if arg == "--attached" {
            cli.attached = true;
        } else if arg == "--index-profile" {
            cli.index_profile = Some(PathBuf::from("."));
        } else if let Some(path) = arg.strip_prefix("--index-profile=") {
            if path.is_empty() {
                return Err("--index-profile= requires a path".into());
            }
            cli.index_profile = Some(PathBuf::from(path));
        } else if let Some(n) = arg.strip_prefix("--profile-top=") {
            cli.profile_top = positive(n, "--profile-top")?;
        } else if let Some(path) = arg.strip_prefix("--profile-out=") {
            if path.is_empty() {
                return Err("--profile-out= requires a path".into());
            }
            cli.profile_out = Some(PathBuf::from(path));
        } else if let Some(n) = arg.strip_prefix("--profile-repeat=") {
            cli.profile_repeat = positive(n, "--profile-repeat")?;
        } else if arg == "--profile-real-paths" {
            cli.profile_real_paths = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown argument: {arg}"));
        } else {
            positionals.push(arg);
        }
        i += 1;
    }
    // A positional subcommand (the first non-flag arg); an unknown name is
    // a hard error (usage on stderr, exit 2).
    if let Some(first) = positionals.first() {
        let sub = Subcommand::parse(first)
            .ok_or_else(|| format!("unknown subcommand: {first}"))?;
        // W11: extra positionals on a verb that takes none are a usage
        // error (the second token used to be silently dropped). And a
        // required-arg verb with no argument is a usage error too.
        let rest = positionals[1..].join(" ");
        if sub.takes_no_arg() && !rest.is_empty() {
            return Err(format!(
                "`{}` takes no arguments (got `{}`)",
                sub.name(),
                rest
            ));
        }
        if sub.requires_arg() && rest.is_empty() {
            return Err(format!("`{}` needs an argument", sub.name()));
        }
        cli.subcommand = Some(sub);
        cli.sub_args = positionals[1..].to_vec();
    }
    // W10: validate every `-c` line HERE so a bad trailing token (e.g.
    // `-c 'state --json'`) is the SAME usage error as a bad positional —
    // printed to stderr with exit 2, not a run-time `anyhow` error (exit 1).
    // `split_cmd_line` already enforces `validate_verb_args`.
    for raw in &cli.commands {
        split_cmd_line(raw)?;
    }
    Ok(cli)
}

fn parse_args() -> CliArgs {
    match parse_cli(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(msg) => {
            eprintln!("redline: {msg}");
            usage();
            std::process::exit(2);
        }
    }
}

/// Whether a subcommand is a printing verb (it emits to stdout). The
/// mutation verbs (`do`/`key`/`keys`/`text`/`quit`) do not print.
fn is_printing(sub: Subcommand) -> bool {
    matches!(
        sub,
        Subcommand::State | Subcommand::Quote | Subcommand::Buffer | Subcommand::Notes
    )
}

/// Run a `do` line's `rest` through the registry: first whitespace run
/// separates the command name from its optional argument.
fn run_do(session: &mut session::Session, rest: &str) -> anyhow::Result<()> {
    let (name, arg) = split_do(rest);
    session
        .do_command(&name, arg)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

/// The control-socket client path (issue 019-02).
///
/// When a socket is requested (`--socket` given, `$REDLINE_SOCKET` set, or
/// `--attached` given), the headless subcommands are routed over the wire to
/// the LIVE TUI instead of building a local `Session`. Returns `None` when NO
/// socket was requested (the caller takes the plain headless path). A
/// `--socket` that was given explicitly and cannot be connected is a HARD
/// error; an env-only socket that is missing falls back to headless. With
/// `--attached` (W15) ANY missing socket is a hard error — the flag promises
/// the command runs against a live session, so silently falling back would
/// lie about what happened.
fn try_attached(cli: &CliArgs) -> Option<anyhow::Result<()>> {
    let env_socket = std::env::var("REDLINE_SOCKET").ok().filter(|s| !s.is_empty());
    let explicit = cli.socket.clone();
    if explicit.is_none() && env_socket.is_none() && !cli.attached {
        return None;
    }
    let sock = control::socket_path(explicit.as_deref());
    // Liveness probe: a missing socket with an explicit `--socket` (or with
    // `--attached`) is an error; an env-only socket falls back to headless.
    if control::client::try_connect(&sock).is_err() {
        if explicit.is_some() || cli.attached {
            return Some(Err(anyhow::anyhow!(
                "cannot connect to control socket {}",
                sock.display()
            )));
        }
        return None;
    }
    Some(run_attached(cli, &sock))
}

/// The wire request for a parsed `(Subcommand, rest)` line.
fn request_for(id: u64, sub: Subcommand, rest: &str, cli: &CliArgs) -> control::Request {
    let args = match sub {
        Subcommand::Do => {
            let (name, arg) = split_do(rest);
            match arg {
                Some(a) => serde_json::json!({"name": name, "arg": a}),
                None => serde_json::json!({"name": name}),
            }
        }
        Subcommand::Key => serde_json::json!({"token": rest.trim()}),
        Subcommand::Keys => serde_json::json!({"seq": rest.trim()}),
        Subcommand::Text => serde_json::json!({"text": rest}),
        Subcommand::Buffer => match parse_range(rest) {
            Some((s, e)) => serde_json::json!({"start": s, "end": e}),
            None => serde_json::json!({}),
        },
        Subcommand::Notes => serde_json::json!({"plain": cli.notes_plain}),
        // Sugar verbs (019-03): the raw argument (empty for the no-arg
        // verbs), routed through `Session::verb` on the server side.
        _ if sub.is_sugar() => serde_json::json!({"arg": rest.trim()}),
        _ => serde_json::json!({}),
    };
    control::Request {
        id,
        cmd: sub.name().to_string(),
        args,
    }
}

/// Print a printing verb's response exactly as the headless path does.
fn emit_response(sub: Subcommand, json: bool, resp: &control::Response) -> anyhow::Result<()> {
    let result = resp
        .result
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("empty result for a printing verb"))?;
    match sub {
        Subcommand::State => {
            if json {
                println!("{}", serde_json::to_string(result)?);
            } else {
                // The server sends the serialized `Snapshot`; rebuild it and
                // render the same human text form the headless path uses.
                let snap: session::Snapshot = serde_json::from_value(result.clone())?;
                println!("{}", snap.to_text());
            }
        }
        Subcommand::Quote | Subcommand::Buffer | Subcommand::Notes => {
            print!("{}", result["text"].as_str().unwrap_or_default());
        }
        _ => {}
    }
    Ok(())
}

fn run_attached(cli: &CliArgs, sock: &Path) -> anyhow::Result<()> {
    let cmds: Vec<(Subcommand, String)> = cli
        .commands
        .iter()
        .map(|raw| split_cmd_line(raw))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let n = cmds.len();
    let has_positional = cli.subcommand.is_some();
    let mut id = 0u64;
    let mut send = |sub: Subcommand, rest: &str, emit: bool| -> anyhow::Result<()> {
        id += 1;
        let req = request_for(id, sub, rest, cli);
        let resp = control::client::send(sock, &req)?;
        if !resp.ok {
            let err = resp.error.map(|e| e.message).unwrap_or_default();
            anyhow::bail!("{err}");
        }
        if emit {
            emit_response(sub, cli.json, &resp)?;
        }
        Ok(())
    };
    for (i, (sub, rest)) in cmds.iter().enumerate() {
        let is_last = i + 1 == n;
        let emit = is_last && !has_positional && is_printing(*sub);
        send(*sub, rest, emit)?;
    }
    if let Some(sub) = cli.subcommand {
        let rest = cli.sub_args.join(" ");
        let printed = is_printing(sub);
        send(sub, &rest, printed)?;
        // W17: `quit` is a terminal, non-printing verb — do NOT dump state
        // after it. Every other non-printing verb falls back to `state`.
        if !printed && sub != Subcommand::Quit {
            send(Subcommand::State, "", true)?;
        }
    } else {
        let last = cmds.last().map(|(s, _)| *s);
        let last_prints = last.map(is_printing).unwrap_or(false);
        if !last_prints && last != Some(Subcommand::Quit) {
            send(Subcommand::State, "", true)?;
        }
    }
    Ok(())
}

/// Execute a subcommand. `emit` selects whether a printing verb writes to
/// stdout (mutation verbs always run). Returns whether anything printed.
fn perform(
    session: &mut session::Session,
    sub: Subcommand,
    rest: &str,
    notes_plain: bool,
    json: bool,
    emit: bool,
) -> anyhow::Result<bool> {
    match sub {
        Subcommand::State => {
            if emit {
                let snap = session.snapshot();
                if json {
                    println!("{}", serde_json::to_string(&snap)?);
                } else {
                    println!("{}", snap.to_text());
                }
            }
            Ok(emit)
        }
        Subcommand::Quote => {
            if emit {
                println!("{}", session.quote());
            }
            Ok(emit)
        }
        Subcommand::Buffer => {
            if emit {
                print!("{}", session.buffer_text(parse_range(rest)));
            }
            Ok(emit)
        }
        Subcommand::Notes => {
            if emit {
                print!("{}", session.notes(notes_plain));
            }
            Ok(emit)
        }
        Subcommand::Do => {
            run_do(session, rest)?;
            Ok(false)
        }
        Subcommand::Key => {
            session.key(rest.trim()).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(false)
        }
        Subcommand::Keys => {
            session.keys(rest.trim()).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(false)
        }
        Subcommand::Text => {
            session.text(rest).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(false)
        }
        Subcommand::Quit => {
            // Set the quit request through the same registry command the TUI
            // uses (W12: no `std::process::exit` deep inside `perform`); the
            // caller sees `session.quit_requested()` and stops.
            let _ = session.do_command("quit", None);
            Ok(false)
        }
        // ── sugar verbs (019-03): all route through `Session::verb` ──
        Subcommand::Open
        | Subcommand::Goto
        | Subcommand::Jump
        | Subcommand::Search
        | Subcommand::Isearch
        | Subcommand::Select
        | Subcommand::Confirm
        | Subcommand::Cancel
        | Subcommand::Annotate
        | Subcommand::Magit => {
            let arg = if rest.trim().is_empty() {
                None
            } else {
                Some(rest.trim())
            };
            session
                .verb(sub.name(), arg)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(false)
        }
    }
}

/// The headless subcommand runner (issue 019-01b). Rule (documented):
/// every `-c` line runs IN ORDER (mutation verbs always; a printing verb
/// only emits when it is the final `-c` line and no positional overrides
/// it). Then the positional subcommand runs if given; if the positional is
/// a mutation verb that prints nothing, `state` follows as the default
/// emitter. With no positional: the final `-c` line emits if it was a
/// printing verb, else `state`. With neither `-c` nor a positional: `state`.
fn run_headless(cli: &CliArgs) -> anyhow::Result<()> {
    let mut session = session::Session::at(&cli.project, cli.width, cli.height);
    // Parse all -c lines up front (a bad verb is a hard error).
    let cmds: Vec<(Subcommand, String)> = cli
        .commands
        .iter()
        .map(|raw| split_cmd_line(raw))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let n = cmds.len();
    let has_positional = cli.subcommand.is_some();
    for (i, (sub, rest)) in cmds.iter().enumerate() {
        let is_last = i + 1 == n;
        // A printing -c verb emits only when it is the final line and no
        // positional subcommand will override it.
        let emit = is_last && !has_positional && is_printing(*sub);
        perform(&mut session, *sub, rest, cli.notes_plain, cli.json, emit)?;
        // W17: `quit` is terminal and non-printing — stop here without a
        // state dump.
        if session.quit_requested() {
            return Ok(());
        }
    }
    if let Some(sub) = cli.subcommand {
        let rest = cli.sub_args.join(" ");
        let printed = perform(&mut session, sub, &rest, cli.notes_plain, cli.json, true)?;
        if sub == Subcommand::Quit || session.quit_requested() {
            return Ok(());
        }
        if !printed {
            perform(
                &mut session,
                Subcommand::State,
                "",
                cli.notes_plain,
                cli.json,
                true,
            )?;
        }
    } else {
        let last = cmds.last().map(|(s, _)| *s);
        let last_prints = last.map(is_printing).unwrap_or(false);
        if !last_prints && last != Some(Subcommand::Quit) {
            perform(
                &mut session,
                Subcommand::State,
                "",
                cli.notes_plain,
                cli.json,
                true,
            )?;
        }
    }
    Ok(())
}

#[cfg(unix)]
mod tty_stdout {
    //! plan 005 issue 03: when stdout is redirected (not a tty), the TUI's
    //! frames and cursor escapes would land on the redirected fd and bury
    //! the quit-dump (verified live: `redline > notes.txt` captured ~10 KB
    //! of alt-screen/CUP/SGR escapes). The fd-level fix: save the original
    //! stdout fd (`dup`) and point fd 1 at `/dev/tty` (`dup2`) BEFORE the
    //! render loop starts. iocraft/crossterm then write to the tty (their
    //! `std::io::stdout()` writes go through fd 1, which is now the tty),
    //! and the saved fd is used only for the dump. The iocraft
    //! `unstable-output-streams` route is flagged as "has crossterm
    //! caveats" (skills), and `libc` is only a transitive dependency (not
    //! re-exported by crossterm), so the two syscalls are declared here
    //! instead of adding a dependency.
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::fd::RawFd;
    use std::os::unix::io::AsRawFd;

    // SAFETY: POSIX libc declarations with exact C-ABI signatures
    // (`int dup(int)`, `int dup2(int, int)`, `int close(int)`); each call
    // site below states the fd-ownership invariant that makes ITS call
    // sound.
    unsafe extern "C" {
        fn dup(fd: RawFd) -> RawFd;
        fn dup2(oldfd: RawFd, newfd: RawFd) -> RawFd;
        fn close(fd: RawFd) -> i32;
    }

    /// The rerouted state: where the quit-dump goes. `tty` stays open for
    /// the process lifetime — after a successful `dup2`, fd 1 IS a dup of
    /// it and must remain valid under the render loop.
    pub struct ReroutedStdout {
        /// The original (redirected) stdout fd — the dump target when the
        /// `dup2` succeeded; `None` when it failed, in which case the dump
        /// goes to `/dev/tty` (fd 1 still carries the TUI frames and must
        /// not receive the dump).
        pub original_stdout: Option<RawFd>,
        /// The reason the `dup2` failed (reporting only; `None` on success).
        pub dup2_error: Option<std::io::Error>,
        /// `/dev/tty`, kept open for the process lifetime — after a
        /// successful `dup2`, fd 1 is a dup of it and must stay valid
        /// under the render loop.
        pub tty: File,
    }

    /// Save fd 1 and point it at `/dev/tty`. The caller must verify
    /// stdout is not a tty first (this is a no-op mistake on a tty).
    pub fn reroute_stdout_to_tty() -> io::Result<ReroutedStdout> {
        // SAFETY: fd 1 is the process's stdout — valid for the process's
        // whole life (opened before `main`). `dup` returns a NEW fd that
        // is uniquely ours: every path below either closes it (the two
        // `close(original)` calls) or transfers unique ownership to a
        // `File::from_raw_fd` — never both.
        let original = unsafe { dup(1) };
        if original < 0 {
            return Err(io::Error::last_os_error());
        }
        // /dev/tty must be opened READ-WRITE: after the `dup2` it IS fd 1,
        // and an O_RDONLY fd 1 fails every TUI write with EBADF (verified
        // live via strace — the std `File::open` O_RDONLY path). crossterm
        // opens its own /dev/tty handle O_RDWR|O_NONBLOCK for input; this
        // one is the output side.
        let tty = match OpenOptions::new().read(true).write(true).open("/dev/tty") {
            Ok(tty) => tty,
            // No controlling tty: undo the `dup` and report — the dump
            // will be skipped rather than corrupt the redirected stream.
            Err(e) => {
                // SAFETY: `original` came from our `dup` above and has not
                // been closed yet. This path (the `/dev/tty` open failed)
                // is mutually exclusive with the other two paths that end
                // this function — the `dup2`-failure path and the
                // success path (which hands the fd to `from_raw_fd`) — so
                // this is its only close. A double close would be UB, not
                // a leak.
                let _ = unsafe { close(original) };
                return Err(e);
            }
        };
        // SAFETY: `tty` is an owned `File` we just opened, so its raw fd
        // is valid, and fd 1 is the process's own fd — a legal `dup2`
        // target (replaced atomically). OWNERSHIP CONSEQUENCE, the real
        // invariant here: on success, fd 1 *is* a dup of `tty`, so the
        // render loop's writes (which go through fd 1) only work while
        // `tty` stays open — it is kept in `ReroutedStdout` and must
        // outlive the render loop to process exit.
        let dup2 = unsafe { dup2(tty.as_raw_fd(), 1) };
        if dup2 < 0 {
            // The tty fd stays open (closed at process exit); report and
            // degrade: the dump will go to /dev/tty instead. The saved
            // original fd is now dead weight (nothing will write to it,
            // and `original_stdout: None` means `emit_dump` never sees
            // it) — close it rather than leak it for the process life.
            // SAFETY: `original` came from our `dup` and has not been
            // closed yet. This path is mutually exclusive with the other
            // two paths that end this function (the open-failure path
            // already returned; the success path keeps the fd), so this
            // is its only close — a double close would be UB, not a leak.
            let _ = unsafe { close(original) };
            Ok(ReroutedStdout {
                original_stdout: None,
                dup2_error: Some(io::Error::last_os_error()),
                tty,
            })
        } else {
            Ok(ReroutedStdout {
                original_stdout: Some(original),
                dup2_error: None,
                tty,
            })
        }
    }
}

/// Where the quit-dump is written (plan 005 issue 03).
enum DumpSink {
    /// stdout was a tty at startup: the dump prints to stdout after the
    /// alternate screen exits (the terminal is restored by then).
    Stdout,
    /// stdout was redirected: fd 1 was rerouted to /dev/tty before the
    /// render loop; the dump goes to the saved fd (or /dev/tty on reroute
    /// failure). The whole variant is unix-only: on non-unix targets the
    /// reroute machinery (dup/dup2, /dev/tty) does not exist, and the
    /// match in `emit_dump` stays exhaustive with Stdout/Skipped alone.
    #[cfg(unix)]
    Rerouted(tty_stdout::ReroutedStdout),
    /// stdout was redirected but /dev/tty could not be opened: the dump is
    /// SKIPPED (report + skip rather than corrupt the redirected stream).
    Skipped,
}

/// Set up the quit-dump sink (plan 005 issue 03). Must run BEFORE the
/// render loop starts (the `dup2` must land first).
fn init_dump_sink() -> DumpSink {
    if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return DumpSink::Stdout;
    }
    #[cfg(unix)]
    {
        match tty_stdout::reroute_stdout_to_tty() {
            Ok(rerouted) => {
                if let Some(err) = &rerouted.dup2_error {
                    eprintln!(
                        "redline: stdout is not a tty and the stdout->/dev/tty reroute failed ({err}); the annotation dump will be printed to /dev/tty at quit"
                    );
                }
                DumpSink::Rerouted(rerouted)
            }
            Err(e) => {
                eprintln!(
                    "redline: stdout is not a tty and /dev/tty could not be opened ({e}); the annotation dump is skipped at quit"
                );
                DumpSink::Skipped
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("redline: stdout is not a tty; the annotation dump is skipped at quit");
        DumpSink::Skipped
    }
}

/// Print the quit-dump to its sink (plan 005 issue 03). `dump` is already
/// the full text (empty = nothing to print).
fn emit_dump(dump: &str, sink: &DumpSink) {
    if dump.is_empty() {
        return;
    }
    let ok = match sink {
        DumpSink::Stdout => std::io::stdout().write_all(dump.as_bytes()).is_ok(),
        DumpSink::Skipped => return,
        #[cfg(unix)]
        DumpSink::Rerouted(rerouted) => {
            if let Some(fd) = rerouted.original_stdout {
                use std::os::fd::FromRawFd;
                // SAFETY: `fd` is the `original_stdout` from our `dup`;
                // on the success path it was never closed (the two
                // `close(original)` calls return before this sink ever
                // exists), so the fd is still valid. `from_raw_fd` takes
                // UNIQUE ownership: this `File` is now the only owner and
                // the only thing that will close it (on drop, after the
                // dump) — closing the fd anywhere else would be a double
                // close (UB).
                unsafe { std::fs::File::from_raw_fd(fd) }
                    .write_all(dump.as_bytes())
                    .is_ok()
            } else {
                // dup2 failed: the dump goes to /dev/tty (fd 1 still
                // carries the TUI frames). Write directly to the file —
                // a BufWriter would defer small writes to drop, whose
                // error is ignored, silently losing a failed tty write.
                let mut tty = &rerouted.tty;
                tty.write_all(dump.as_bytes()).is_ok()
            }
        }
    };
    if !ok {
        eprintln!("redline: could not write the annotation dump");
    }
}


fn log_path() -> std::path::PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("redline").join("redline.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("redline.log"))
}

fn init_tracing() -> std::path::PathBuf {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .expect("open log file");
    tracing_subscriber::fmt()
        .with_writer(file)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();
    path
}

/// Terminal restore + panic log, chained to the hook iocraft installed
/// (if any) so a panic never leaves the terminal in raw mode.
fn init_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort: iocraft's render loop enables raw mode; the call
        // fails (silently) when raw mode was never active (pre-loop
        // panics).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = crossterm::terminal::disable_raw_mode();
        }));
        tracing::error!("redline panicked: {}", panic_message(info));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| default(info)));
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    info.payload()
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown panic".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_panic_hook();
    let cli = parse_args();
    // Headless index profiler (the issue-index-profiler task): run BEFORE
    // any terminal/TUI setup (before the IsTerminal check and everything
    // after it) and exit. Works with no tty. When the flag is absent this
    // is a single branch miss.
    if let Some(root) = cli.index_profile.clone() {
        index_profile::run(&index_profile::Options {
            root,
            top: cli.profile_top,
            out: cli.profile_out,
            repeat: cli.profile_repeat,
            real_paths: cli.profile_real_paths,
        })?;
        return Ok(());
    }
    // Headless subcommand mode (issue 019-01b): a positional subcommand or
    // any `-c` line takes the headless path — run BEFORE any terminal/TUI
    // setup and exit. Bare invocation (and `--notes=plain`) is unchanged.
    // 019-02: when a control socket is requested, the subcommands are routed
    // over the wire to the live TUI instead of building a local session.
    if cli.subcommand.is_some() || !cli.commands.is_empty() {
        if let Some(result) = try_attached(&cli) {
            result?;
            return Ok(());
        }
        run_headless(&cli)?;
        return Ok(());
    }
    let log_path = init_tracing();
    tracing::info!(log = %log_path.display(), "redline starting");

    let config: config::Config = config::load_tolerant();
    let mut store = AppStore::new();
    // Measure the terminal height at startup: crossterm emits no Resize
    // event at startup, so the store's default viewport_lines (24) would
    // be wrong on terminals that are not ~27 rows.
    if let Ok((_, h)) = crossterm::terminal::size() {
        store.set_viewport_lines(h.saturating_sub(3) as usize);
    }
    if let Err(err) = config.validate_bindings(&store.registry) {
        tracing::warn!("ignoring some key-bindings: {err}");
    }
    if let Err(err) = store.apply_config(&config) {
        tracing::warn!("key-binding overrides not fully applied: {err}");
    }
    theme::set_current(store.theme().clone());
    tracing::info!(bindings = config.key_bindings.len(), "config loaded");

    // Start the file watcher for the initial project (plan decision #7). It
    // runs on this runtime and publishes project-change events to the bus the
    // FileView / git-status subscribers consume. Stops cleanly on quit (the
    // store's Drop signals the watcher to tear down).
    store.start_watcher();

    // Start the background symbol index build (issue 05). Runs on a
    // rayon thread via spawn_blocking; the UI drain installs each result
    // into the store. No-op in plain unit tests (no runtime).
    // Capture the index-bus receiver BEFORE the job starts: a tokio watch
    // send with zero receivers discards the event, which would lose the full
    // index result and leave the status line stuck at "indexing 0/N" forever.
    let index_rx = store.index_bus.subscribe();
    store.set_index_rx(index_rx);
    store.start_indexing();

    // The store lives in the element context; the root component reads and
    // updates it there (including the SearchBus drain, issue 06). Keep a
    // handle so we can stop the watcher cleanly at shutdown (before the
    // runtime tears down).
    let store_handle = std::sync::Arc::new(std::sync::Mutex::new(store));
    // 019-02: start the control socket so an external client can drive the
    // live session. A bind failure is warned (not fatal): the TUI must still
    // run. The socket wraps the SAME `store_handle` the render loop owns.
    {
        let sock = control::socket_path(cli.socket.as_deref());
        let serve_store = std::sync::Arc::clone(&store_handle);
        let serve_sock = sock.clone();
        tokio::spawn(async move {
            if let Err(e) = control::serve(serve_sock.clone(), serve_store).await {
                tracing::warn!(path = %serve_sock.display(), "control socket unavailable: {e}");
                eprintln!("redline: control socket unavailable: {e}");
            }
        });
        // The path is deterministic; print it now (the bind itself is async).
        eprintln!("redline: control socket at {}", sock.display());
        tracing::info!(path = %sock.display(), "control socket at");
    }
    let mut app = element! {
        ContextProvider(value: Context::owned(std::sync::Arc::clone(&store_handle))) {
            Root
        }
    };
    // iocraft's fullscreen loop exits on Ctrl+C by default -- which would
    // swallow every C-c-prefixed binding (C-c p ..., the commit editor's
    // C-c C-c/C-c C-k) before the keymap ever sees the key. Opt out: C-c is
    // ours (a mode prefix); C-x C-c is the quit path (bare `q` is a no-op
    // close-view on the root buffer view, issue 05 finding 5).
    // plan 005 issue 03: set up the quit-dump sink BEFORE the loop starts
    // (the stdout->/dev/tty dup2 must land first so the render loop's
    // frames never reach a redirected stdout).
    let dump_sink = init_dump_sink();
    app.fullscreen().ignore_ctrl_c().await?;
    // Clean watcher shutdown: take the watcher out (brief lock) and await its
    // teardown WITHOUT holding the store lock across the await (the runtime
    // must not block on a std Mutex while a task is running).
    let watcher = store_handle.lock().unwrap().take_watcher();
    if let Some(mut w) = watcher {
        w.stop_and_wait().await;
    }
    // W16: remove the control-socket file on clean exit. The path is
    // deterministic from the same inputs that chose it at startup; a failed
    // unlink (already gone) is harmless.
    {
        let sock = control::socket_path(cli.socket.as_deref());
        let _ = std::fs::remove_file(&sock);
    }
    tracing::info!("redline exited cleanly");
    // plan 005 issue 03: the quit-dump — printed AFTER the alternate screen
    // has exited and the watcher has stopped, from the FINAL store state
    // (any annotations saved during quit save-prompts are included). Never
    // printed before this point (it must not interleave with the TUI).
    {
        let mut store = store_handle.lock().unwrap();
        let items = store.annotations_for_dump();
        let root = store
            .project
            .as_ref()
            .map(|p| p.root.display().to_string())
            .unwrap_or_default();
        let dump = app::store::format_notes_dump(
            &items,
            &root,
            cli.notes_plain,
        );
        emit_dump(&dump, &dump_sink);
    }
    Ok(())
}

#[cfg(test)]
mod cli_tests {
    //! Arg parsing (plan 005 issue 03 extended by the index-profiler
    //! task): `--notes=plain` still parses, the headless profiler flags
    //! parse, and GENUINELY unknown args remain a hard error.
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_default_to_the_tui() {
        let cli = parse_cli(strs(&[])).unwrap();
        assert!(!cli.notes_plain);
        assert!(cli.index_profile.is_none());
        assert_eq!(cli.profile_top, CliArgs::DEFAULT_TOP);
        assert_eq!(cli.profile_repeat, 1);
        assert!(cli.profile_out.is_none());
        assert!(!cli.profile_real_paths);
    }

    #[test]
    fn notes_plain_still_accepted() {
        let cli = parse_cli(strs(&["--notes=plain"])).unwrap();
        assert!(cli.notes_plain);
        assert!(cli.index_profile.is_none());
    }

    #[test]
    fn index_profile_flag_and_path() {
        assert_eq!(
            parse_cli(strs(&["--index-profile"])).unwrap().index_profile,
            Some(PathBuf::from("."))
        );
        assert_eq!(
            parse_cli(strs(&["--index-profile=./big-project"]))
                .unwrap()
                .index_profile,
            Some(PathBuf::from("./big-project"))
        );
        assert!(parse_cli(strs(&["--index-profile="])).is_err());
    }

    #[test]
    fn profile_options_parse() {
        let cli = parse_cli(strs(&[
            "--index-profile",
            "--profile-top=10",
            "--profile-out=/tmp/index.csv",
            "--profile-repeat=3",
            "--profile-real-paths",
        ]))
        .unwrap();
        assert_eq!(cli.profile_top, 10);
        assert_eq!(cli.profile_repeat, 3);
        assert_eq!(
            cli.profile_out.as_deref(),
            Some(std::path::Path::new("/tmp/index.csv"))
        );
        assert!(cli.profile_real_paths);
    }

    #[test]
    fn profile_options_validate_values() {
        assert!(parse_cli(strs(&["--profile-top=0"])).is_err());
        assert!(parse_cli(strs(&["--profile-top=abc"])).is_err());
        assert!(parse_cli(strs(&["--profile-repeat=0"])).is_err());
        assert!(parse_cli(strs(&["--profile-out="])).is_err());
        // Valid: top=1 is allowed (just one row).
        assert_eq!(parse_cli(strs(&["--profile-top=1"])).unwrap().profile_top, 1);
    }

    #[test]
    fn genuinely_unknown_args_still_hard_error() {
        let err = parse_cli(strs(&["--bogus"])).unwrap_err();
        assert!(err.contains("--bogus"), "error names the arg: {err}");
        let err = parse_cli(strs(&["--notes=plain", "--nope"]))
            .unwrap_err();
        assert!(err.contains("--nope"), "error names the arg: {err}");
    }

    #[test]
    fn bare_invocation_unchanged() {
        // No subcommand, no -c: the TUI path (nothing headless).
        let cli = parse_cli(strs(&[])).unwrap();
        assert!(cli.subcommand.is_none());
        assert!(cli.commands.is_empty());
        let cli = parse_cli(strs(&["--notes=plain"])).unwrap();
        assert!(cli.subcommand.is_none());
        assert!(cli.commands.is_empty());
        assert!(cli.notes_plain);
    }

    #[test]
    fn subcommand_parses() {
        let cli = parse_cli(strs(&["state"])).unwrap();
        assert_eq!(cli.subcommand, Some(Subcommand::State));
        assert!(cli.sub_args.is_empty());
        assert!(!cli.json);

        let cli = parse_cli(strs(&["--json", "state"])).unwrap();
        assert_eq!(cli.subcommand, Some(Subcommand::State));
        assert!(cli.json);

        let cli = parse_cli(strs(&[
            "--project",
            ".",
            "-c",
            "open src/main.rs:5",
            "state",
        ]))
        .unwrap();
        assert_eq!(cli.project, PathBuf::from("."));
        assert_eq!(cli.commands, vec!["open src/main.rs:5".to_string()]);
        assert_eq!(cli.subcommand, Some(Subcommand::State));

        // A positional `do NAME ARG`: rest is joined from sub_args.
        let cli = parse_cli(strs(&["do", "find-file"])).unwrap();
        assert_eq!(cli.subcommand, Some(Subcommand::Do));
        assert_eq!(cli.sub_args, vec!["find-file".to_string()]);

        // Defaults.
        assert_eq!(cli.width, CliArgs::DEFAULT_WIDTH);
        let cli = parse_cli(strs(&["--width", "120", "--height", "40", "state"])).unwrap();
        assert_eq!(cli.width, 120);
        assert_eq!(cli.height, 40);
    }

    #[test]
    fn unknown_subcommand_is_usage_error() {
        assert!(parse_cli(strs(&["bogus"])).is_err());
        assert!(parse_cli(strs(&["--json", "bogus"])).is_err());
    }

    #[test]
    fn cmd_line_grammar_splits_on_first_whitespace() {
        assert_eq!(
            split_cmd_line("do find-file").unwrap(),
            (Subcommand::Do, "find-file".to_string())
        );
        assert_eq!(
            split_cmd_line("text Cargo.toml").unwrap(),
            (Subcommand::Text, "Cargo.toml".to_string())
        );
        assert_eq!(
            split_cmd_line("key RET").unwrap(),
            (Subcommand::Key, "RET".to_string())
        );
        assert_eq!(
            split_cmd_line("state").unwrap(),
            (Subcommand::State, String::new())
        );
        assert!(split_cmd_line("bogus x").is_err());
    }

    // ── W10: a no-arg verb rejects trailing flags/tokens in `-c` ────────
    #[test]
    fn w10_no_arg_verb_rejects_trailing_token_in_c() {
        let err = split_cmd_line("state --json").unwrap_err();
        assert!(err.contains("takes no arguments"), "{err}");
        assert!(split_cmd_line("quote extra").is_err());
        assert!(split_cmd_line("cancel now").is_err());
        assert!(split_cmd_line("magit --json").is_err());
        // A plain state still parses.
        assert!(split_cmd_line("state").is_ok());
    }

    // ── W11: extra positionals on a no-arg verb are a usage error ───────
    #[test]
    fn w11_extra_positional_is_a_usage_error() {
        let err = parse_cli(strs(&["state", "state"])).unwrap_err();
        assert!(err.contains("takes no arguments"), "{err}");
        assert!(parse_cli(strs(&["quote", "x"])).is_err());
        // A verb that takes an argument still accepts one.
        assert!(parse_cli(strs(&["goto", "10"])).is_ok());
    }

    // ── W15: `--attached` parses ─────────────────────────────────────────
    #[test]
    fn w15_attached_flag_parses() {
        let cli = parse_cli(strs(&["--attached", "state"])).unwrap();
        assert!(cli.attached);
        assert_eq!(cli.subcommand, Some(Subcommand::State));
        let cli = parse_cli(strs(&["state"])).unwrap();
        assert!(!cli.attached);
    }

    // ── Sugar verbs parse at the top level and in `-c` ──────────────────
    #[test]
    fn sugar_verbs_parse() {
        assert_eq!(parse_cli(strs(&["open", "src/main.rs:5"])).unwrap().subcommand, Some(Subcommand::Open));
        assert_eq!(parse_cli(strs(&["goto", "10"])).unwrap().sub_args, vec!["10".to_string()]);
        assert!(parse_cli(strs(&["jump", "alpha"])).is_ok());
        assert!(parse_cli(strs(&["search", "needle"])).is_ok());
        assert!(parse_cli(strs(&["isearch", "needle"])).is_ok());
        assert!(parse_cli(strs(&["select", "next"])).is_ok());
        assert!(parse_cli(strs(&["select"])).is_ok());
        assert!(parse_cli(strs(&["confirm"])).is_ok());
        assert!(parse_cli(strs(&["cancel"])).is_ok());
        assert!(parse_cli(strs(&["annotate", "a note"])).is_ok());
        assert!(parse_cli(strs(&["magit"])).is_ok());
        // A required-arg verb with no arg is an error.
        assert!(parse_cli(strs(&["open"])).is_err());
        assert!(parse_cli(strs(&["goto"])).is_err());
        // And the -c form.
        assert_eq!(
            split_cmd_line("open src/main.rs:5").unwrap(),
            (Subcommand::Open, "src/main.rs:5".to_string())
        );
        assert!(split_cmd_line("goto").is_err());
    }
}
