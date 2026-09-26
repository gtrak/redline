// issue-non-rust-receiver-resolution: the two halves, pinned at the store
// level.
//
// Part 1 (correctness): a non-Rust dotted-path token whose HEAD is a
// LOCAL VALUE (a variable the file declares) must stay BARE — it is not
// a package name and must never be handed to a provider as one (the
// 011-06 P1's `pip install df` shape: `df` from `df = read_data()`).
// A module/package head (an import binding, or a name the file declares
// nowhere) keeps the 011-06 whole-path upgrade, byte-for-byte.
//
// Part 2 (safety): a provider's fetch-on-demand install step (pip install
// / npm install / go mod download) is gated behind an explicit, visible
// `y`/`n` confirmation. A DECLINE (or an absent/declined UI) refuses the
// fetch; an ACCEPT runs exactly one install command (the stub toolchain
// on PATH logs its argv — no real install ever happens).
//
// Rust and C are byte-for-byte unchanged (pins below).

use super::*;

/// The point-line token extractor over a FULL multi-line `source`
/// (issue-non-rust-receiver-resolution: the receiver classification is
/// whole-file, so the extractor's `line` index + rope inputs matter —
/// the single-line `satp` helper above pins `line == 0`, rope == text).
/// P2-5, gate: the extraction is rope-backed (CRLF files included —
/// ropey's line primitives are the authoritative offset source).
fn satp_src(lang: LanguageId, line: usize, col: usize, source: &str) -> Option<(String, String)> {
    let line_text = source
        .lines()
        .nth(line)
        .unwrap_or("")
        .to_string();
    let rope = ropey::Rope::from(source);
    AppStore::symbol_at_point(lang, &line_text, line, col, &rope)
}

// ── Part 1: receiver resolution (the local-variable gate) ───────────────

/// Python: `df` is a module-level VALUE (`df = read_data()`), so
/// `df.head` stays BARE — the head is not a package. (011-06 P1: pre-fix
/// this upgraded to `df.head` and pip tried to install `df`.)
#[test]
fn receiver_python_local_head_stays_bare() {
    let src = "df = read_data()\nprint(df.head)\n";
    // line 1, `df.head`: `print(df.head)` — `head` starts at col 11.
    assert_eq!(
        satp_src(LanguageId::Python, 1, 12, src),
        Some(("head".into(), "head".into())),
        "a local-variable head must stay bare"
    );
}

/// Python: a `for` target is a local value too.
#[test]
fn receiver_python_for_target_head_stays_bare() {
    let src = "for df in xs:\n    print(df.head)\n";
    // line 1: `    print(df.head)` — `head` at col 15.
    assert_eq!(
        satp_src(LanguageId::Python, 1, 16, src),
        Some(("head".into(), "head".into()))
    );
}

/// Python: a function parameter (`df`) is a local value.
#[test]
fn receiver_python_param_head_stays_bare() {
    let src = "def f(self, df):\n    return df.head\n";
    // line 1: `    return df.head` — `head` at cols 14-17.
    assert_eq!(
        satp_src(LanguageId::Python, 1, 16, src),
        Some(("head".into(), "head".into()))
    );
}

/// Python: `self` is a parameter → local value → bare (the method's
/// own field, never a package).
#[test]
fn receiver_python_self_head_stays_bare() {
    let src = "class A:\n    def m(self):\n        return self.x\n";
    // line 2: `        return self.x` — `x` at col 20.
    assert_eq!(satp_src(LanguageId::Python, 2, 20, src), Some(("x".into(), "x".into())));
}

/// Python: a module head from an IMPORT is NOT a local value — the
/// 011-06 whole-path upgrade stands (`json.dumps`).
#[test]
fn receiver_python_import_head_still_upgrades() {
    let src = "import json\nprint(json.dumps)\n";
    // line 1: `print(json.dumps)` — `dumps` at col 11.
    assert_eq!(
        satp_src(LanguageId::Python, 1, 12, src),
        Some(("dumps".into(), "json.dumps".into()))
    );
}

/// Python: a head the file declares NOWHERE is not a local value — the
/// 011-06 upgrade stands (the fetch gate, not the receiver gate, guards
/// the install).
#[test]
fn receiver_python_undeclared_head_still_upgrades() {
    let src = "df.head\n";
    // line 0: `df.head` — `head` at col 5.
    assert_eq!(
        satp_src(LanguageId::Python, 0, 5, src),
        Some(("head".into(), "df.head".into()))
    );
}

/// JS/TS: a `const` value is local → `df.head` stays bare.
#[test]
fn receiver_js_local_head_stays_bare() {
    let src = "const df = {head: 1};\ndf.head\n";
    // line 1: `df.head` — `head` at col 3.
    assert_eq!(satp_src(LanguageId::JavaScript, 1, 3, src), Some(("head".into(), "head".into())));
}

/// JS/TS: `this` is never a package.
#[test]
fn receiver_js_this_head_stays_bare() {
    let src = "class A { m() { return this.x; } }\n";
    // `this.x`: `x` at col 28.
    assert_eq!(satp_src(LanguageId::JavaScript, 0, 28, src), Some(("x".into(), "x".into())));
}

/// JS/TS: an arrow-function parameter is a local value.
#[test]
fn receiver_js_arrow_param_head_stays_bare() {
    let src = "const f = (df) => df.head;\n";
    // `df.head`: `head` at col 22.
    assert_eq!(satp_src(LanguageId::JavaScript, 0, 22, src), Some(("head".into(), "head".into())));
}

/// JS/TS: a default-imported module is a PACKAGE head — the 011-06
/// upgrade stands (the provider resolves it; the gate guards the fetch).
#[test]
fn receiver_js_import_head_still_upgrades() {
    let src = "import * as fakelib from \"fakelib\";\nfakelib.apply(5);\n";
    // line 1: `fakelib.apply(5)` — `apply` at col 10.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 1, 10, src),
        Some(("apply".into(), "fakelib.apply".into()))
    );
}

/// Go: a short-var-declared local (`x := …`) makes `x.New` BARE — a
/// method/field selector on a value, never a package.
#[test]
fn receiver_go_local_selector_head_stays_bare() {
    let src = "package main\n\nfunc f() {\n\tx := makeV()\n\t_ = x.New\n}\n";
    // line 4: `\t_ = x.New` — `New` at col 8.
    assert_eq!(satp_src(LanguageId::Go, 4, 8, src), Some(("New".into(), "New".into())));
}

/// Go: a function parameter is a local value.
#[test]
fn receiver_go_param_head_stays_bare() {
    let src = "package main\n\nfunc f(x T) {\n\t_ = x.New\n}\n";
    // line 3: `\t_ = x.New` — `New` at col 8.
    assert_eq!(satp_src(LanguageId::Go, 3, 8, src), Some(("New".into(), "New".into())));
}

/// Go: an imported package's qualified type is a PACKAGE head — the
/// 011-06 upgrade stands (`pkg.T`).
#[test]
fn receiver_go_import_qualified_type_head_still_upgrades() {
    let src = "package main\n\nimport \"pkg\"\n\nvar v pkg.T\n";
    // line 4: `var v pkg.T` — `T` at col 10.
    assert_eq!(satp_src(LanguageId::Go, 4, 10, src), Some(("T".into(), "pkg.T".into())));
}

/// Rust: a `.` field access stays BARE byte-for-byte (the 010-01
/// pre-steps own it; the receiver gate never touches Rust).
#[test]
fn receiver_rust_dotted_stays_bare() {
    assert_eq!(
        satp_src(LanguageId::Rust, 0, 5, "obj.field\n"),
        Some(("field".into(), "field".into()))
    );
}

/// C: no fetching provider — the 011-06 whole-path upgrade stands
/// byte-for-byte (`ns.member`), unaffected by the receiver gate.
#[test]
fn receiver_c_dotted_still_upgrades() {
    assert_eq!(
        satp_src(LanguageId::C, 0, 5, "ns.member\n"),
        Some(("member".into(), "ns.member".into()))
    );
}

// ── P1-1, gate: CRLF line endings (the lane's line_start_byte drift) ──────

/// P1-1, gate: CRLF equality. The lane computed the line's start by
/// `source.lines()` + one byte per ending — `lines()` strips the `\r`, so
/// a CRLF file drifted ONE byte per preceding CRLF line; the node then
/// landed on the WRONG line, the guards failed, and EVERY dotted M-. in a
/// CRLF file lost the 011-06 upgrade (gate's live evidence: LF → jump into
/// `json/__init__.py`; CRLF → `no provider resolution for 'dumps'`;
/// JS `fakelib.apply` → `apply`). The extraction is now line-local, so
/// the CRLF spelling must equal the LF spelling, both directions.
///
/// RED-PROOF SHAPE: the use line sits AFTER enough CRLF-terminated lines
/// that the lane's drifted offset lands OFF the point's line (on a
/// terminator, or inside a PRECEDING line's own dotted container) — with
/// the lane's arithmetic this test fails (import head: no container at
/// the drifted byte → bare; local head: the preceding line's
/// `json.dumps` container → a false upgrade); line-local it passes.
#[test]
fn receiver_crlf_equals_lf_both_directions() {
    // Python, import head (the upgrade direction — the gate's `dumps`
    // repro): the drifted byte falls on the line's own CRLF terminator →
    // the lane keeps the bare token, the line-local path upgrades.
    let lf = "import json\n".to_string() + &"a = 0\n".repeat(12) + "json.dumps\n";
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(
        satp_src(LanguageId::Python, 13, 5, &crlf),
        satp_src(LanguageId::Python, 13, 5, &lf),
        "CRLF must equal the LF spelling (import head)"
    );
    assert_eq!(
        satp_src(LanguageId::Python, 13, 5, &crlf),
        Some(("dumps".into(), "json.dumps".into()))
    );
    // Python, local head (the gate direction): the drifted byte lands
    // inside the PRECEDING line's `json.dumps` container — the lane
    // UPGRADED a local-head use to a package-shaped token (the 011-06
    // P1's `pip install` shape, CRLF-only); the line-local path keeps it
    // bare (`df` is a local value).
    let lf_local = "import json\n".to_string()
        + "df = read_data()\n"
        + &"x = json.dumps\n".repeat(11)
        + "df.head\n";
    let crlf_local = lf_local.replace('\n', "\r\n");
    assert_eq!(
        satp_src(LanguageId::Python, 13, 4, &crlf_local),
        satp_src(LanguageId::Python, 13, 4, &lf_local),
        "CRLF must equal the LF spelling (local head)"
    );
    assert_eq!(
        satp_src(LanguageId::Python, 13, 4, &crlf_local),
        Some(("head".into(), "head".into()))
    );
    // JS (the gate's second live repro: `fakelib.apply` stayed bare on
    // CRLF) — enough preceding lines that the drifted byte leaves the
    // container.
    let lf_js = "import * as fakelib from \"fakelib\";\n".to_string()
        + &"z = 1;\n".repeat(12)
        + "fakelib.apply(5);\n";
    let crlf_js = lf_js.replace('\n', "\r\n");
    assert_eq!(
        satp_src(LanguageId::JavaScript, 13, 10, &crlf_js),
        Some(("apply".into(), "fakelib.apply".into()))
    );
    // Go: the CRLF spelling equals the LF spelling (equivalence guard —
    // the two legs above are the red-proof ones).
    let lf_go = "package main\n\nimport \"pkg\"\n\nvar v pkg.T\n";
    let crlf_go = lf_go.replace('\n', "\r\n");
    assert_eq!(
        satp_src(LanguageId::Go, 4, 10, &crlf_go),
        satp_src(LanguageId::Go, 4, 10, lf_go),
        "CRLF must equal the LF spelling (Go)"
    );
    assert_eq!(
        satp_src(LanguageId::Go, 4, 10, &crlf_go),
        Some(("T".into(), "pkg.T".into()))
    );
}

// ── P2-1, gate: JS named-function / rest / method parameters ─────────────

/// P2-1, gate: a NAMED function's parameter is a local value — the lane's
/// `function_declaration` arm pruned the `formal_parameters`, so this was
/// false (the token upgraded and would have reached the provider).
#[test]
fn receiver_js_named_function_param_head_stays_bare() {
    let src = "function f(df) { return df.head; }\n";
    // line 0: `df.head` — `head` at cols 27-30.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 0, 28, src),
        Some(("head".into(), "head".into()))
    );
}

/// P2-1, gate: a REST parameter (`...df`) is a local value too.
#[test]
fn receiver_js_rest_param_head_stays_bare() {
    let src = "function f(a, ...df) { return df.slice; }\n";
    // line 0: `df.slice` — `slice` at cols 33-36.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 0, 34, src),
        Some(("slice".into(), "slice".into()))
    );
}

/// P2-1, gate: a method's parameter is a local value.
#[test]
fn receiver_js_method_param_head_stays_bare() {
    let src = "class C { m(df) { return df.head; } }\n";
    // line 0: `df.head` — `head` at cols 28-31.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 0, 29, src),
        Some(("head".into(), "head".into()))
    );
}

/// P2-1 over-approximation guard: a head the file declares NOWHERE still
/// upgrades (a named function's params must not leak out of the function
/// — `other` is not a parameter of `f`, so `other.x` stays package-shaped).
#[test]
fn receiver_js_undeclared_head_still_upgrades() {
    let src = "function f() { return other.x; }\n";
    // line 0: `other.x` — `x` at col 28.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 0, 28, src),
        Some(("x".into(), "other.x".into()))
    );
}

// ── P2-2, gate: JS destructuring shorthand ───────────────────────────────

/// P2-2, gate: `const { df } = x` — the shorthand carries the name, so
/// `df.head` stays bare (the lane's identifier-only pattern walk missed
/// `shorthand_property_identifier_pattern`).
#[test]
fn receiver_js_destructure_shorthand_head_stays_bare() {
    let src = "const { df } = x;\ndf.head\n";
    // line 1: `df.head` — `head` at cols 3-6.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 1, 3, src),
        Some(("head".into(), "head".into()))
    );
}

/// P2-2 over-approximation guard: a NAMED IMPORT head still upgrades —
/// the import binding is a package head, and destructuring carriers must
/// not swallow it (`df` here comes from the import, not a destructure).
#[test]
fn receiver_js_named_import_head_still_upgrades() {
    let src = "import { df } from \"p\";\ndf.head\n";
    // line 1: `df.head` — `head` at cols 3-6.
    assert_eq!(
        satp_src(LanguageId::JavaScript, 1, 3, src),
        Some(("head".into(), "df.head".into()))
    );
}

// ── P2-3, gate: Python with/except-as, walrus, lambda, comprehension ────

/// P2-3, gate: `with open(…) as df` — the alias is a local value.
#[test]
fn receiver_python_with_as_alias_head_stays_bare() {
    let src = "def f():\n    with open('f') as df:\n        return df.read()\n";
    // line 2: `df.read()` — `read` at cols 18-21.
    assert_eq!(
        satp_src(LanguageId::Python, 2, 20, src),
        Some(("read".into(), "read".into()))
    );
}

/// P2-3, gate: `except E as df` — the alias is a local value.
#[test]
fn receiver_python_except_as_alias_head_stays_bare() {
    let src = "def f():\n    try:\n        pass\n    except ValueError as df:\n        pass\n    print(df.msg)\n";
    // line 5: `    print(df.msg)` — `msg` at cols 13-15.
    assert_eq!(
        satp_src(LanguageId::Python, 5, 14, src),
        Some(("msg".into(), "msg".into()))
    );
}

/// P2-3, gate: a walrus target (`if (df := read())`) is a local value in
/// the ENCLOSING scope (Python's rule — the position is irrelevant).
#[test]
fn receiver_python_walrus_head_stays_bare() {
    let src = "def f():\n    if (df := read()):\n        pass\n    print(df.head)\n";
    // line 3: `    print(df.head)` — `head` at cols 13-16.
    assert_eq!(
        satp_src(LanguageId::Python, 3, 14, src),
        Some(("head".into(), "head".into()))
    );
}

/// P2-3, gate: a lambda parameter is local in its body.
#[test]
fn receiver_python_lambda_param_head_stays_bare() {
    let src = "g = lambda df: df.head\n";
    // line 0: `df.head` — `head` at cols 18-21.
    assert_eq!(
        satp_src(LanguageId::Python, 0, 19, src),
        Some(("head".into(), "head".into()))
    );
}

/// P2-3, gate: a comprehension target is local AT A POINT INSIDE the
/// comprehension (the generator scope: `df` in `df.head`).
#[test]
fn receiver_python_comprehension_target_head_stays_bare() {
    let src = "def f(ys):\n    return [df.head for df in ys]\n";
    // line 1: `df.head` (the body) — `head` at cols 15-18.
    assert_eq!(
        satp_src(LanguageId::Python, 1, 16, src),
        Some(("head".into(), "head".into()))
    );
}

/// P2-3 over-approximation guard (the conditional half): a comprehension
/// target is NOT the enclosing function's binding (Python 3 generator
/// scope) — `df` outside the comprehension stays the 011-06 upgrade.
#[test]
fn receiver_python_comprehension_target_not_local_outside() {
    let src = "def f(ys):\n    h = [d.head for d in ys]\n    return df.head\n";
    // line 2: `df.head` — `head` at cols 14-17.
    assert_eq!(
        satp_src(LanguageId::Python, 2, 15, src),
        Some(("head".into(), "df.head".into()))
    );
}

// ── P2-4, gate: Go closures (the grammar kind is `func_literal`) ─────────

/// P2-4, gate: a closure's short-var local is a local value (`x` in
/// `x.New`) — the lane's `function_literal` kind never matched
/// `func_literal`, so the closure's scope was never seen.
#[test]
fn receiver_go_closure_local_head_stays_bare() {
    let src = "package main\n\nfunc f() {\n\tg := func() { x := 1; _ = x.New }\n\t_ = g\n}\n";
    // line 3: `x.New` — `New` at cols 29-31.
    assert_eq!(
        satp_src(LanguageId::Go, 3, 29, src),
        Some(("New".into(), "New".into()))
    );
}

/// P2-4, gate: a closure's parameter is a local value in its body.
#[test]
fn receiver_go_closure_param_head_stays_bare() {
    let src = "package main\n\nfunc f() {\n\tg := func(x T) { _ = x.New }\n\t_ = g\n}\n";
    // line 3: `x.New` — `New` at cols 24-26.
    assert_eq!(
        satp_src(LanguageId::Go, 3, 24, src),
        Some(("New".into(), "New".into()))
    );
}

/// P2-4 over-approximation guard: an UNDECLARED Go head still upgrades
/// (the closure prune must not swallow package-qualified references).
#[test]
fn receiver_go_undeclared_head_still_upgrades() {
    let src = "package main\n\nfunc f() {\n\t_ = other.New\n}\n";
    // line 3: `other.New` — `New` at col 11.
    assert_eq!(
        satp_src(LanguageId::Go, 3, 11, src),
        Some(("New".into(), "other.New".into()))
    );
}

// ── P3-4, gate: the fetch prompt survives a stale event ────────────────────

/// P3-4, gate: a STALE resolve event lands while the current generation's
/// request is still in flight — the stale branch clears the `resolving`
/// INDICATOR (006-02b item 3: a stale send can win the watch slot and
/// lose the current event). The current generation's fetch ask then
/// arrives: pre-fix, `apply_fetch_prompt` keyed liveness on `resolving`
/// (now `None`) and DECLINED the live ask with NO banner — the operator
/// never saw the prompt and the request reported a refusal. Post-fix,
/// liveness is the `resolve_in_flight` flag: the banner still arms.
#[test]
fn fetch_prompt_survives_stale_event_clearing_the_indicator() {
    let (mut s, _dir) = store_with_index(&[(
        "main.py",
        "import json\n\nprint(json.dumps)\n",
    )]);
    // A live in-flight request of the current generation (the shape
    // `start_symbol_resolution` leaves behind — no runtime here, so no
    // provider really runs; the state pair is what matters). Pin a
    // generation ≥ 1 (the fresh store's is 0 — a stale leg needs a
    // lower one).
    let gen_tag = 5;
    s.resolve_generation = gen_tag;
    s.resolving = Some(("resolving `json.dumps`…".to_string(), gen_tag));
    s.resolve_in_flight = true;
    // The stale event lands first: the indicator clears.
    s.apply_resolve_event(&crate::app::store::ResolveEvent {
        generation: gen_tag - 1,
        symbol: "stale".to_string(),
        source: None,
        error: Some("stale".to_string()),
    });
    assert!(s.resolving.is_none(), "the stale branch clears the indicator");
    // The current generation's ask then arrives: the banner arms (pre-fix
    // this ask was silently declined — the silent-failure path).
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    s.apply_fetch_prompt(crate::app::store::FetchConfirmAsk {
        generation: gen_tag,
        command: "pip install json".to_string(),
        from_file: "main.py".to_string(),
        reply: reply_tx,
    });
    assert!(
        s.fetch_confirm_active(),
        "the live ask must arm the banner even after a stale event cleared the indicator"
    );
    assert!(
        reply_rx.try_recv().is_err(),
        "no decline was sent while the banner is up"
    );
}

/// P3-4 control: an ask for the current generation while NO request is in
/// flight (never started, or already applied) still declines — the banner
/// never arms for a dead request (the pre-P3-4 behavior for that case is
/// preserved; only the in-flight case changed).
#[test]
fn fetch_prompt_declines_when_no_request_in_flight() {
    let (mut s, _dir) = store_with_index(&[("main.py", "x = 1\n")]);
    let gen_tag = s.resolve_generation;
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    s.apply_fetch_prompt(crate::app::store::FetchConfirmAsk {
        generation: gen_tag,
        command: "pip install x".to_string(),
        from_file: "main.py".to_string(),
        reply: reply_tx,
    });
    assert!(!s.fetch_confirm_active(), "no banner for a dead request");
    assert!(
        !reply_rx.try_recv().unwrap_or(true),
        "the ask is declined"
    );
}

// ── Part 1 at the M-. seam (no background runtime needed) ───────────────

/// The P1's end-to-end shape: M-. on `df.head` where `df` is a local
/// variable — the resolver receives the BARE `head` (in-project lookup),
/// never the package-shaped `df.head` (which the Python provider would
/// have tried to `pip install`).
#[test]
fn mdot_python_local_variable_reaches_resolver_bare() {
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "df = read_data()\nprint(df.head)\n",
        ),
    ]);
    s.open_path("main.py");
    s.set_point(1, 12, 12); // cursor inside `head`
    s.xref_find_definitions();
    assert!(
        s.message.contains("no provider resolution for `head`"),
        "the resolver got the BARE local field, got: {}",
        s.message
    );
    assert!(
        !s.message.contains("df.head"),
        "the local-variable head must NOT reach the provider as a package: {}",
        s.message
    );
}

/// The contrasting shape: M-. on `json.dumps` (an import module head) —
/// the resolver receives the WHOLE path (the 011-06 upgrade survives the
/// receiver gate).
#[test]
fn mdot_python_import_module_reaches_resolver_dotted() {
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "import json\n\nprint(json.dumps)\n",
        ),
    ]);
    s.open_path("main.py");
    s.set_point(2, 12, 12); // cursor inside `dumps`
    s.xref_find_definitions();
    assert!(
        s.message.contains("no provider resolution for `json.dumps`"),
        "the resolver got the dotted import path, got: {}",
        s.message
    );
}

// ── Part 2: the fetch-confirmation gate (store level, stub toolchain) ──

/// A stub `pip` on PATH that logs its argv (one line per invocation) and
/// exits 1. The guard restores PATH on drop. No real install ever runs.
struct PipStub {
    log: std::path::PathBuf,
    old_path: Option<std::ffi::OsString>,
}

impl PipStub {
    fn install() -> (Self, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = dir.path().join("pip.log");
        // The provider runs `pip3` (or a workspace venv's `pip` when one
        // exists — this fixture has none); cover both names.
        let stub_body = format!(
            "#!/bin/sh\nfor a; do printf '%s ' \"$a\" >> \"{}\"; done; printf '\\n' >> \"{}\"\nexit 1\n",
            log.display(),
            log.display()
        );
        for name in ["pip", "pip3"] {
            let stub = bin.join(name);
            std::fs::write(&stub, &stub_body).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    &stub,
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
        }
        let old_path = std::env::var_os("PATH");
        let new_path = format!(
            "{}{}",
            bin.display(),
            std::env::var("PATH").map(|p| format!(":{p}")).unwrap_or_default()
        );
        // SAFETY: test-only PATH override. The process environment is
        // process-global, so std's contract is that a writer must exclude
        // ANY concurrent reader (not just readers of THIS variable): the
        // fetch tests below hold `crate::ENV_LOCK` for their whole body,
        // which serializes every env reader/writer in this test binary
        // (single process-global environment, one lock).
        unsafe { std::env::set_var("PATH", new_path) };
        (
            Self {
                log,
                old_path,
            },
            dir,
        )
    }

    fn invocations(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

impl Drop for PipStub {
    fn drop(&mut self) {
        // SAFETY: same as install — restore the prior PATH.
        unsafe {
            match self.old_path.take() {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}

/// Wait (bounded) for the in-flight resolve event of `gen_tag`, applying it.
/// `rx` is the long-lived subscriber the test holds from BEFORE the request
/// started (Root's drain holds its subscription for the session's lifetime
/// — the watch `send` needs at least one live receiver to land, and a
/// fresh subscribe only sees values sent AFTER it).
fn wait_resolve_event(
    s: &mut AppStore,
    rx: &mut tokio::sync::watch::Receiver<crate::app::store::ResolveEvent>,
    gen_tag: usize,
) -> crate::app::store::ResolveEvent {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let ev = rx.borrow().clone();
        if ev.generation == gen_tag && (ev.source.is_some() || ev.error.is_some()) {
            s.apply_resolve_event(&ev);
            return ev;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "resolve event for generation {gen_tag} did not land (latest: {ev:?})"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The gate's safe default: the operator DECLINES (`n`) — the provider
/// refuses the fetch (naming the exact command that was NOT run) and the
/// stub pip is never invoked.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // the ENV_LOCK must span the whole body (incl. the `.await`s) so parallel env-mutating tests never interleave (the stub PATH override lives under the same process-global environment)
async fn fetch_declined_refuses_pip_install() {
    let _env_guard = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (stub, _keep) = PipStub::install();
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "import redline_test_pkg\n\nprint(redline_test_pkg.get)\n",
        ),
    ]);
    // The test plays Root's drain: take the confirm receiver and hold the
    // long-lived resolve-bus subscriber (Root holds its for the session).
    let mut rx = s.fetch_confirm_rx().unwrap();
    let mut resolve_rx = s.resolve_bus.subscribe();
    s.open_path("main.py");
    s.set_point(2, 22, 22); // cursor inside `get`
    s.xref_find_definitions();
    let gen_tag = s.resolve_generation;
    // The provider's blocked ask arrives on the bus.
    let ask = tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv())
        .await
        .expect("the fetch ask must arrive")
        .expect("bus open");
    assert_eq!(ask.generation, gen_tag, "the ask belongs to the in-flight request");
    assert_eq!(
        ask.command, "pip install redline_test_pkg",
        "the banner shows the EXACT command"
    );
    // Root's drain applies the ask → the banner arms, showing the EXACT
    // command and the file that implied it (requirement: confirm shows
    // both — `y` never approves something unseen).
    s.apply_fetch_prompt(ask);
    assert!(s.fetch_confirm_active(), "the y/n banner is on screen");
    assert!(
        s.message.contains("fetch on demand: pip install redline_test_pkg (from main.py) (y/n)?"),
        "the banner shows the exact command and its origin, got: {}",
        s.message
    );
    // The operator declines.
    s.key_event(key("n"));
    assert!(!s.fetch_confirm_active());
    // The resolve event is a MISS (the refusal bailed the provider); the
    // stub pip never ran — that is the P1's safety property.
    let ev = wait_resolve_event(&mut s, &mut resolve_rx, gen_tag);
    assert!(ev.source.is_none(), "a declined fetch must not land a source");
    assert!(ev.error.is_some(), "the miss reports a failure, got: {:?}", ev.error);
    // F2 (plan 017 audit): the refusal's own text is no longer swallowed
    // by the chain-level generic — the MINIBUFFER names the decision the
    // operator just made (which command was NOT run), so a refusal is
    // not indistinguishable from a bare "symbol not found".
    assert!(
        s.message.contains("install refused"),
        "the refusal reaches the minibuffer: {}", s.message
    );
    assert!(
        s.message.contains("declined at the fetch confirmation"),
        "the refusal's reason (the gate's own text): {}", s.message
    );
    assert!(
        s.message.contains("pip install redline_test_pkg"),
        "the exact command that was NOT run: {}", s.message
    );
    assert_eq!(stub.invocations(), Vec::<String>::new(), "no pip invocation on decline");
}

/// The sanctioned path: the operator ACCEPTS (`y`) — the provider runs
/// EXACTLY ONE install command (the stub logs `install requests`), then
/// bails honestly when the module is still missing (the stub fetches
/// nothing).
#[tokio::test]
#[allow(clippy::await_holding_lock)] // the ENV_LOCK must span the whole body (incl. the `.await`s) so parallel env-mutating tests never interleave (the stub PATH override lives under the same process-global environment)
async fn fetch_accepted_runs_stub_pip_exactly_once() {
    let _env_guard = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (stub, _keep) = PipStub::install();
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "import redline_test_pkg\n\nprint(redline_test_pkg.get)\n",
        ),
    ]);
    let mut rx = s.fetch_confirm_rx().unwrap();
    let mut resolve_rx = s.resolve_bus.subscribe();
    s.open_path("main.py");
    s.set_point(2, 22, 22);
    s.xref_find_definitions();
    let gen_tag = s.resolve_generation;
    let ask = tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv())
        .await
        .expect("the fetch ask must arrive")
        .unwrap();
    s.apply_fetch_prompt(ask);
    assert!(s.fetch_confirm_active());
    s.key_event(key("y"));
    assert!(!s.fetch_confirm_active());
    // The provider ran the stub (exit 1 → the fetch "failed"/missed):
    // the event is a MISS, and the stub ran EXACTLY ONCE with the exact
    // argv (the sanctioned install actually happened — the gate let it
    // through because the operator approved).
    let ev = wait_resolve_event(&mut s, &mut resolve_rx, gen_tag);
    assert!(ev.error.is_some(), "the honest miss after the failed stub install, got: {:?}", ev.error);
    assert_eq!(
        stub.invocations(),
        vec!["install redline_test_pkg".to_string()],
        "exactly one install command, with the exact argv"
    );
}

/// The gate must not over-block: an UNDECLARED head (no local binding, no
/// import) still reaches the fetch ask (the P1's `df`-as-module shape for
/// a genuinely unknown name — the operator gets the choice).
#[tokio::test]
#[allow(clippy::await_holding_lock)] // the ENV_LOCK must span the whole body (incl. the `.await`s) so parallel env-mutating tests never interleave (the stub PATH override lives under the same process-global environment)
async fn fetch_undeclared_head_still_asks() {
    let _env_guard = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (_stub, _keep) = PipStub::install();
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "redline_test_pkg.get('x')\n",
        ),
    ]);
    let mut rx = s.fetch_confirm_rx().unwrap();
    s.open_path("main.py");
    s.set_point(0, 19, 19); // cursor inside `get`
    s.xref_find_definitions();
    let ask = tokio::time::timeout(std::time::Duration::from_secs(15), rx.recv())
        .await
        .expect("an undeclared head must STILL ask (the gate is not over-broad)")
        .unwrap();
    assert_eq!(ask.command, "pip install redline_test_pkg");
    // Decline (the test only pins that the ask arrives).
    s.apply_fetch_prompt(ask);
    s.key_event(key("n"));
}

/// A LOCAL-variable head never asks (it never reaches the fetch step):
/// M-. on `df.head` with `df = …` stays in the project index path (the
/// no-runtime miss message names the BARE field — no fetch, no ask).
#[test]
fn fetch_local_head_never_asks() {
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "df = read_data()\nprint(df.head)\n",
        ),
    ]);
    s.open_path("main.py");
    s.set_point(1, 12, 12);
    s.xref_find_definitions();
    assert!(
        s.message.contains("no provider resolution for `head`"),
        "the bare field took the index path, got: {}",
        s.message
    );
    // No fetch ask was published (the bus is still whole, nothing queued).
    let rx = s.fetch_confirm_rx().unwrap();
    drop(rx); // the receiver stays with the store; nothing to drain
}

/// A stale ask (its request is no longer on screen — e.g. the resolve
/// already finished without a fetch, or a new M-. superseded it) is
/// declined immediately: the blocked hook unblocks with `false`, the
/// install never runs unconfirmed.
#[test]
fn fetch_stale_ask_is_declined_without_a_banner() {
    let (mut s, _dir) = store_with_index(&[
        (
            "main.py",
            "df = read_data()\nprint(df.head)\n",
        ),
    ]);
    // No in-flight resolve (resolving is None after the no-runtime miss):
    // an ask for generation 2 is stale → declined on apply.
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let gen_tag = s.resolve_generation;
    s.apply_fetch_prompt(crate::app::store::FetchConfirmAsk {
        generation: gen_tag,
        command: "pip install df".to_string(),
        from_file: "main.py".to_string(),
        reply: reply_tx,
    });
    assert!(!s.fetch_confirm_active(), "a stale ask must not arm the banner");
    assert!(
        !reply_rx.recv().unwrap_or(true),
        "the superseded provider's blocked hook is unblocked with a decline"
    );
}
