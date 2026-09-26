use super::*;

// jump-column-landings: the landings that still travelled a bare line number
// (col-0 `set_point_line`) now land on the definition's name column wherever
// the index records one, and degrade honestly to column 0 where it does not.
// Every test below uses a fixture whose symbol sits at a NONZERO column so
// the col-0 regression cannot pass (the isearch lane's lesson: its fixture's
// match sat at column 0, so the test could not discriminate at all).

/// In-project Xref picker RET: the candidate row carries only "file:line";
/// the definition's `start_byte` is re-read from the project index and the
/// landing sits on the name, not the line start. `pub fn target` puts the
/// name at col 7 (nonzero) — the old col-0 landing would fail this.
#[test]
fn xref_picker_in_project_lands_name_column() {
    let (mut s, _dir) = store_with_index(&[
        ("src/main.rs", "fn main() { target(); }\n"),
        ("src/lib.rs", "pub fn target() {}\npub fn other() {}\n"),
    ]);
    s.open_path("src/main.rs");
    // Line 0: `fn main() { target(); }` — cursor on `target` (col 12).
    s.set_point(0, 12, 12);
    s.xref_find_definitions();
    assert!(s.picker_open(), "cross-file unique: picker");
    assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
    assert!(
        s.picker_filtered()[0].0.name.starts_with("src/lib.rs:"),
        "the preselected row is target's definition: {:?}",
        s.picker_filtered()[0].0.name
    );
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the definition's line");
    assert_eq!(
        s.point_col(),
        7,
        "the name's column (pub fn |target), not col 0 (msg: {})",
        s.message
    );
}

/// Crate-relative (external) Xref picker RET: the same column re-read, but
/// from the OWNING crate's index (006-03 keying) — the picker is
/// crate-rooted, so `definition_start_byte` consults the crate index, not
/// the project's. `pub fn target` → col 7 (nonzero).
#[test]
fn xref_picker_crate_relative_lands_name_column() {
    let (mut s, _dir, root) = store_with_crate_index(&[
        ("src/lib.rs", "pub fn target() {}\n"),
        ("src/main.rs", "fn main() { target(); }\n"),
    ]);
    s.open_external_path(&root.path().join("src/main.rs")).unwrap();
    // Line 0: `fn main() { target(); }` — cursor on `target` (col 12).
    s.set_point(0, 12, 12);
    s.xref_find_definitions();
    assert!(s.picker_open(), "cross-crate-file: picker");
    assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
    assert!(
        s.picker_filtered()[0].0.name.starts_with("src/lib.rs:"),
        "crate-relative row: {:?}",
        s.picker_filtered()[0].0.name
    );
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the definition's line");
    assert_eq!(
        s.point_col(),
        7,
        "crate-relative landing on the name (msg: {})",
        s.message
    );
}

/// The external/crate silent jump (`ExternalXrefOutcome::Jump`, xref.rs): a
/// same-crate-file unique M-. lands without a picker. The outcome enum
/// carries only a line; the column is re-read from the crate index (the
/// enum change is the deferred follow-up — re-querying needs no mod.rs edit).
/// `pub fn target` → col 7 (nonzero).
#[test]
fn external_crate_unique_jump_lands_name_column() {
    let (mut s, _dir, root) = store_with_crate_index(&[(
        "src/lib.rs",
        "pub fn target() {}\npub fn caller() { target(); }\n",
    )]);
    s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
    // Line 1: `pub fn caller() { target(); }` — cursor on `target` (col 18).
    s.set_point(1, 18, 18);
    s.xref_find_definitions();
    assert!(!s.picker_open(), "unique same-crate-file: silent jump, no picker");
    assert_eq!(s.point_line(), 0, "the definition's line");
    assert_eq!(
        s.point_col(),
        7,
        "external/crate landing on the name (msg: {})",
        s.message
    );
}

/// Imenu picker RET: the candidate encodes "symbol:line" (the line only);
/// the current buffer's outline is re-read for that (name, line) symbol's
/// recorded `start_byte`. `    pub fn |new` puts the name at col 11
/// (nonzero) — the old col-0 landing would fail this.
#[test]
fn imenu_picker_lands_name_column() {
    let (mut s, _dir) = store_with_index(&[(
        "src/lib.rs",
        "pub struct Foo { a: i32 }\nimpl Foo {\n    pub fn new() -> Self { Self { a: 0 } }\n}\npub fn free() {}\n",
    )]);
    s.open_path("src/lib.rs");
    s.open_imenu();
    assert_eq!(s.picker_kind(), Some(PickerKind::Imenu));
    // Select the `new` row (0-based line 2, col 11) wherever it sits.
    let names: Vec<_> = s
        .picker_filtered()
        .iter()
        .map(|(c, _)| c.name.clone())
        .collect();
    let idx = names
        .iter()
        .position(|n| n == "new:3")
        .unwrap_or_else(|| panic!("the `new` row is present: {names:?}"));
    for _ in 0..idx {
        s.picker_select_next();
    }
    assert_eq!(s.picker_selected(), idx);
    s.run_selected();
    assert_eq!(s.point_line(), 2, "the `new` method's line (0-based)");
    assert_eq!(
        s.point_col(),
        11,
        "the `new` name's column (    pub fn |new), not col 0 (msg: {})",
        s.message
    );
}

/// The tooling/resolver landing (navigation/mod.rs — the path `M-.` on a
/// function usually takes in a Rust project, the cargo provider): the
/// provider pins a line, no column; the app-side refinement locates the
/// resolved item's name (`tokio::spawn` → `spawn`) on that line and lands on
/// its first char column. `pub fn |spawn` → col 7 (nonzero) — the old
/// col-0 landing is exactly the user's reported defect.
#[tokio::test]
async fn tooling_landing_refines_to_name_column() {
    let (mut s, _dir) = store_with_index(&[
        ("src/main.rs", "tokio::spawn(f);\n"),
    ]);
    let ext = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(ext.path(), "extern crate dep;\npub fn spawn<F>(f: F) {}\n").unwrap();
    s.open_path("src/main.rs");
    s.set_point(0, 2, 2); // cursor on `tokio`
    s.xref_find_definitions();
    assert_eq!(s.resolve_generation, 2, "xref supersede bump + start bump");
    let event = ResolveEvent {
        generation: 2,
        symbol: "tokio::spawn".into(),
        source: Some(ResolvedSource {
            file: ext.path().to_path_buf(),
            source_root: ext.path().parent().unwrap().to_path_buf(),
            external: true,
            line: Some(2),
        }),
        error: None,
    };
    s.apply_resolve_event(&event);
    assert!(s.picker_open(), "the tooling hit joins the picker");
    assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
    s.run_selected(); // RET on the preselected tooling row
    assert_eq!(s.point_line(), 1, "the resolved line (0-based)");
    assert_eq!(
        s.point_col(),
        7,
        "the `spawn` name's column (pub fn |spawn), not col 0 (msg: {})",
        s.message
    );
}

/// Multibyte, byte→char: the definition's line starts with a doc attribute
/// holding a multibyte char — `target` sits at BYTE 17 but CHAR 16. A byte
/// landing would put the cursor one char PAST the name; the col-0 landing
/// (the regression) at 0. This asserts the landing is a CHAR column via
/// `try_byte_to_line_col`.
#[test]
fn xref_picker_multibyte_lands_char_column() {
    let (mut s, _dir) = store_with_index(&[
        ("src/main.rs", "fn main() { target(); }\n"),
        ("src/lib.rs", "#[doc = \"é\"] fn target() {}\n"),
    ]);
    s.open_path("src/main.rs");
    s.set_point(0, 12, 12); // cursor on `target`
    s.xref_find_definitions();
    assert!(s.picker_open(), "cross-file unique: picker");
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the definition's line");
    assert_eq!(
        s.point_col(),
        16,
        "char col 16 — not byte 17, not col 0 (msg: {})",
        s.message
    );
}

/// Multibyte, the tooling refinement's char-based scan: the resolved item's
/// line begins with a multibyte char — `spawn` at CHAR 20, not byte 21. A
/// byte-based search would land one char past the name.
#[tokio::test]
async fn tooling_landing_multibyte_lands_char_column() {
    let (mut s, _dir) = store_with_index(&[
        ("src/main.rs", "tokio::spawn(f);\n"),
    ]);
    let ext = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(ext.path(), "#[doc = \"é\"] pub fn spawn() {}\n").unwrap();
    s.open_path("src/main.rs");
    s.set_point(0, 2, 2);
    s.xref_find_definitions();
    let event = ResolveEvent {
        generation: 2,
        symbol: "tokio::spawn".into(),
        source: Some(ResolvedSource {
            file: ext.path().to_path_buf(),
            source_root: ext.path().parent().unwrap().to_path_buf(),
            external: true,
            line: Some(1),
        }),
        error: None,
    };
    s.apply_resolve_event(&event);
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the resolved line (0-based)");
    assert_eq!(
        s.point_col(),
        20,
        "char col 20 — not byte 21, not col 0 (msg: {})",
        s.message
    );
}

/// Honest degradation: the provider pins a line whose text does NOT contain
/// the resolved item as a WHOLE word (`run` only as a substring of
/// `running`). The refinement must land column 0 — never on a substring of a
/// longer identifier (a byte/substring bug would land col 7 on `running`).
#[test]
fn tooling_landing_degrades_to_col0_when_token_absent() {
    let (mut s, _dir) = store_with_index(&[
        ("src/main.rs", "run(1);\n"),
    ]);
    let ext = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(ext.path(), "pub fn running() {}\n").unwrap();
    s.open_path("src/main.rs");
    let source = ResolvedSource {
        file: ext.path().to_path_buf(),
        source_root: ext.path().parent().unwrap().to_path_buf(),
        external: true,
        line: Some(1),
    };
    s.land_tooling_resolved_source(&source, "run");
    assert_eq!(s.point_line(), 0, "the resolved line (0-based)");
    assert_eq!(
        s.point_col(),
        0,
        "no whole-word `run` on the line → honest col 0 (msg: {})",
        s.message
    );
}

/// The refinement's pure seam: whole-word matching (never a substring of a
/// longer identifier), multibyte char-based columns (not byte offsets), and
/// the empty/absent → `None` fallback the caller turns into col 0.
#[test]
fn first_word_column_whole_word_multibyte_and_absent() {
    // The name's column.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "pub fn spawn() {}", "spawn"),
        Some(7)
    );
    // Whole-word only: `run` inside `running` is NOT a match.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "pub fn running() {}", "run"),
        None,
        "a leading word-char disqualifies"
    );
    // A trailing word-char also disqualifies (`respawn`).
    assert_eq!(AppStore::first_word_column(LanguageId::Rust, "respawn()", "spawn"), None);
    // Multibyte prefix: `café spawn` — `spawn` at CHAR 5, not byte 6.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "café spawn()", "spawn"),
        Some(5),
        "a char column, not a byte offset"
    );
    // Absent entirely.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "pub fn spawn() {}", "target"),
        None
    );
    // Empty item: no match (never a spurious col 0 hit).
    assert_eq!(AppStore::first_word_column(LanguageId::Rust, "spawn()", ""), None);
}

/// goto-line (M-g g) is a BARE-LINE landing: it carries a line number with
/// no symbol/column, so it stays at the line start (col 0) — the emacs
/// `goto-line` contract, NOT the jump-to-definition column behavior.
#[test]
fn goto_line_lands_line_start_not_a_symbol() {
    let (mut s, _dir) = store_with_index(&[(
        "src/lib.rs",
        "pub fn target() {}\nfn other() {}\n",
    )]);
    s.open_path("src/lib.rs");
    s.goto_line_start();
    s.goto_line_digit('1');
    s.goto_line_confirm();
    assert_eq!(s.point_line(), 0);
    assert_eq!(
        s.point_col(),
        0,
        "goto-line is a bare-line landing (col 0), not a symbol landing"
    );
}

// ── jump-column-landings follow-up (P2-1/2/3) ──────────────────────────
// P2-1: the annotation record DOES carry a column (Annotation.col, the
// point's column at creation, written to the notes file) — the landing uses
// it instead of col 0. P2-2: a line that hosts two symbols must land on the
// chosen one's name, not the line's first. P2-3: a leading comment or string
// that mentions the resolved item's name is skipped, not landed on.

/// P2-2 (in-project Xref picker): a definition line that hosts two symbols
/// (`fn a() {} fn b() {}`) — M-. on `b` must land on `b`'s name column
/// (col 13), not `a`'s (col 3, the line-only match's result) and not col 0.
/// The picker row's `label` (the resolved symbol's name) is what
/// `definition_start_byte` now matches on.
#[test]
fn xref_picker_two_symbols_one_line_lands_right_one() {
    let (mut s, _dir) = store_with_index(&[
        ("src/main.rs", "fn main() { b(); }\n"),
        ("src/lib.rs", "fn a() {} fn b() {}\n"),
    ]);
    s.open_path("src/main.rs");
    // Line 0: `fn main() { b(); }` — cursor on `b` (col 12).
    s.set_point(0, 12, 12);
    s.xref_find_definitions();
    assert!(s.picker_open(), "cross-file unique: picker");
    assert_eq!(s.picker_kind(), Some(PickerKind::Xref));
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the definition's line");
    assert_eq!(
        s.point_col(),
        13,
        "the `b` name's column (fn a() {{}} fn |b), not `a`'s (3) or col 0 (msg: {})",
        s.message
    );
}

/// P2-2 (external/crate silent jump, xref.rs): the same two-symbol line, but
/// the M-. resolves to a unique same-crate-file def — a silent jump. The
/// definition NAME travels alongside the outcome (`jump_name` from
/// `crate_xref_outcome`), so the re-read matches by (name, line). Lands on
/// `b` (col 13), not `a` (col 3).
#[test]
fn external_crate_two_symbols_one_line_lands_right_one() {
    let (mut s, _dir, root) = store_with_crate_index(&[(
        "src/lib.rs",
        "fn a() {} fn b() {}\nfn caller() { b(); }\n",
    )]);
    s.open_external_path(&root.path().join("src/lib.rs")).unwrap();
    // Line 1: `fn caller() { b(); }` — cursor on `b` (col 14).
    s.set_point(1, 14, 14);
    s.xref_find_definitions();
    assert!(!s.picker_open(), "unique same-crate-file: silent jump, no picker");
    assert_eq!(s.point_line(), 0, "the definition's line");
    assert_eq!(
        s.point_col(),
        13,
        "the `b` name's column (fn a() {{}} fn |b), not `a`'s (3) (msg: {})",
        s.message
    );
}

/// P2-3 (pure seam): `first_word_column` skips a leading inline block comment
/// or string literal that mentions the item's name, landing on the live
/// definition's name. When the name appears ONLY inside a comment/string, it
/// returns `None` (the caller degrades to col 0; never a comment's column).
#[test]
fn first_word_column_skips_leading_comment_and_string_mention() {
    // Leading block comment: the first `spawn` (col 3) is inside `/* … */`;
    // the definition's `spawn` (col 19) is the landing.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "/* spawn */ pub fn spawn() {}", "spawn"),
        Some(19),
        "the leading block-comment mention is skipped"
    );
    // Leading string literal: the first `spawn` (col 9) is inside `\"…\"`;
    // the definition's `spawn` (col 20) is the landing.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "let s = \"spawn\"; fn spawn() {}", "spawn"),
        Some(20),
        "the leading string mention is skipped"
    );
    // Name only inside a comment → no live occurrence → None (honest col 0).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "/* spawn */", "spawn"),
        None,
        "a name only inside a comment is not a landing"
    );
    // Name only inside a string → None.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "let s = \"spawn\";", "spawn"),
        None,
        "a name only inside a string is not a landing"
    );
    // A Rust attribute is NOT a comment (`#[…]`): the name after it lands
    // (guards against masking `#` as a line comment, which would wrongly
    // swallow `#[doc = \"…\"] pub fn spawn()`).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "#[doc = \"x\"] pub fn spawn() {}", "spawn"),
        Some(20),
        "`#[…]` is an attribute, not a comment"
    );
}

/// P2-1 (Annotations picker RET): the record carries a column —
/// `Annotation.col`, the point's column at creation, written to the notes
/// file. The offer encodes only "path:line", so the column is looked up in
/// the notes document (not re-derived) and the landing sits there, not at
/// the line start.
#[test]
fn annotations_picker_lands_recorded_column() {
    let (mut s, dir) = store_with_index(&[("src/lib.rs", "fn target() {}\n")]);
    // A record on (src/lib.rs, line 0) whose recorded column is 7 (the
    // `target` name). Required fields: path, line, anchor, note; `col` is
    // optional metadata carried here.
    std::fs::write(
        dir.path().join(".redline-notes.md"),
        "<!-- redline-annotations:begin -->\n[annotation]\npath: src/lib.rs\nline: 0\ncol: 7\nanchor: fn target() {}\nnote: a note\n<!-- redline-annotations:end -->\n",
    )
    .unwrap();
    s.open_path("src/lib.rs");
    s.open_annotations_picker();
    assert_eq!(s.picker_kind(), Some(PickerKind::Annotations));
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the annotated line");
    assert_eq!(
        s.point_col(),
        7,
        "the recorded column (Annotation.col), not the line start (msg: {})",
        s.message
    );
}

// ── jump-column-landings follow-up (F1/F2) ─────────────────────────────
// F1: the mask UNDER-masked a mention preceding the live definition (a wrong
// column, not col 0) for raw strings, a block-comment continuation, and
// nested comments (Rust's block comments DO nest — a `/*` deepens the run,
// verified against rustc). F2: a bare `'` (a lifetime tick) was treated as a
// string opener, over-masking `&'static str` and landing the live definition
// at col 0.
//
// Follow-up (P2-1/P2-2): the `*/` continuation proxy is gated so it never
// over-masks a live definition whose line also holds a `*/` inside a `//`
// comment or a string (the A2/A3 regressions); and the block-comment loop
// gained a `/*` depth counter so a nested comment masks to its true close.

/// F1 (raw string): a leading raw string that mentions the name is masked —
/// the landing sits on the live definition's name, not the mention. A
/// mention-only raw-string line (no live definition) yields `None` (col 0),
/// not a nonzero column inside the string.
#[test]
fn first_word_column_masks_raw_string_mention() {
    // The raw string `r#"say \"spawn\" now"#` mentions `spawn`; the live def
    // is later. Lands on the def (col 33), not the mention (col 16).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, 
            "let s = r#\"say \"spawn\" now\"#; fn spawn() {}",
            "spawn"
        ),
        Some(33),
        "the raw-string mention (col 16) is skipped; the live def is col 33"
    );
    // A raw string with TWO hash marks (`r##…##`).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, 
            "let s = r##\"spawn\"##; fn spawn() {}",
            "spawn"
        ),
        Some(25),
        "`r##…##` is a raw string; the mention is skipped"
    );
    // Mention-only (no live definition) → `None`, never a column inside the string.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "let s = r#\"spawn\"#;", "spawn"),
        None,
        "a name only inside a raw string is not a landing"
    );
}

/// F1 (block-comment continuation): a line that CONTINUES a block comment
/// opened on a prior line shows a `*/` with no `/*` before it. The leading run
/// to that close is masked, so the landing sits on the live definition, not
/// the mention trapped in the continuation.
#[test]
fn first_word_column_masks_block_comment_continuation() {
    // The `   spawn */` prefix is a continuation of a prior-line `/* …`; the
    // live def is later. Lands on the def (col 19), not the mention (col 3).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "   spawn */ pub fn spawn() {}", "spawn"),
        Some(19),
        "the continuation-run mention (col 3) is skipped; the live def is col 19"
    );
    // A genuine `/*` opener on this line still wins (not a continuation):
    // both comments are masked, the live def (col 23) is returned.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "/* x */ /* spawn */ fn spawn() {}", "spawn"),
        Some(23),
        "a real `/*` on the line is not treated as a continuation"
    );
}

/// P2-1 (regression pin): the `*/` continuation proxy must NOT over-mask a live
/// definition whose line also holds a `*/` inside a `//` comment (A2) or inside
/// a string (A3). Before the gate, the proxy masked from column 0 through that
/// `*/`, sending the landing to col 0 — the exact symptom this lane removes.
#[test]
fn first_word_column_continuation_proxy_respects_strings_and_line_comments() {
    // A2: a trailing `//` comment contains `*/`. The def at col 3 is live.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "fn spawn() {} // closes */ block", "spawn"),
        Some(3),
        "a `*/` inside a `//` comment must not mask the live def (A2)"
    );
    // A3: a later string contains `*/`. The def at col 7 is live.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "pub fn spawn() {} let s = \"*/\";", "spawn"),
        Some(7),
        "a `*/` inside a string must not mask the live def (A3)"
    );
}

/// P2-2 (pin): Rust's block comments NEST — the `spawn` mention inside
/// `/* /* x */ spawn */` sits inside the comment (rustc 1.97.1 confirms the
/// nesting), so it is masked to the true depth-0 close and the landing sits on
/// the live definition, not the mention. A first-`*/`-closes mask would land on
/// the mention (col 11) — wrong.
#[test]
fn first_word_column_masks_nested_block_comment() {
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "/* /* x */ spawn */ pub fn spawn() {}", "spawn"),
        Some(27),
        "the nested comment masks its inner mention; the live def (col 27) lands"
    );
}

/// P3-1 (pin): a backtick string takes a backslash escape (matching the doc),
/// so the escaped backticks do not close it early and the mention inside stays
/// masked. (Non-Rust construct; behaviour ≈ the parent.)
#[test]
fn first_word_column_backtick_string_takes_backslash_escape() {
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "let s = `a\\`spawn\\`b`; fn spawn() {}", "spawn"),
        Some(26),
        "the backslash-escaped backtick string masks its inner mention"
    );
}

/// F2 (regression pin): a lifetime tick (`'static`, `'a`) is NOT a string
/// opener. Before the fix, the bare `'` opened a string that ran to the next
/// `'`, over-masking the live definition to col 0 (or a later mention).
#[test]
fn first_word_column_lifetimes_are_not_string_openers() {
    // `&'static str` before the def: lands on the def (col 30), not col 0.
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, 
            "let v: &'static str = \"x\"; fn spawn() {}",
            "spawn"
        ),
        Some(30),
        "the `'static` tick must not open a string that masks the def"
    );
    // A short lifetime `'a` before the def: lands on the def (col 25).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "let v: &'a str = \"x\"; fn spawn() {}", "spawn"),
        Some(25),
        "the `'a` tick must not open a string"
    );
    // A genuine char literal (`'x'`) before the def is masked and does not
    // over-mask the definition (the fix must not treat a char literal as a
    // lifetime tick either). Lands on the def (col 16).
    assert_eq!(
        AppStore::first_word_column(LanguageId::Rust, "let c = 'x'; fn spawn() {}", "spawn"),
        Some(16),
        "`'x'` is a char literal (masked); the live def at col 16 is untouched"
    );
}

/// F2 end-to-end (the user-facing regression): a tooling landing whose
/// resolved line begins with `&'static str` must land on the live definition's
/// name, not col 0. The base (pre-regression) landed right; HEAD regressed to
/// col 0; this pins the correct column.
#[tokio::test]
async fn tooling_landing_lifetimes_do_not_over_mask() {
    let (mut s, _dir) = store_with_index(&[("src/main.rs", "spawn(f);\n")]);
    let ext = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        ext.path(),
        "let v: &'static str = \"x\"; pub fn spawn() {}\n",
    )
    .unwrap();
    s.open_path("src/main.rs");
    s.set_point(0, 0, 0);
    s.xref_find_definitions();
    let event = ResolveEvent {
        generation: 2,
        symbol: "spawn".into(),
        source: Some(ResolvedSource {
            file: ext.path().to_path_buf(),
            source_root: ext.path().parent().unwrap().to_path_buf(),
            external: true,
            line: Some(1),
        }),
        error: None,
    };
    s.apply_resolve_event(&event);
    s.run_selected();
    assert_eq!(s.point_line(), 0, "the resolved line (0-based)");
    assert_eq!(
        s.point_col(),
        34,
        "pub fn |spawn after a leading `&'static str` — the `'` is a lifetime, not a string (msg: {})",
        s.message
    );
}

// ── issue-external-change-reload F1: open_external_path on a HELD key ─────
// The pre-fix `open_external_path` called `insert_rope` UNCONDITIONALLY:
// a re-landing rebuilt the buffer via `Buffer::new` (mode → Annotation,
// unsaved edits dropped, editable flipped off) — the last unguarded site
// of the unsaved-edits data-loss class. These tests pin the failure mode
// (a dirty Accurate buffer at the key survives the landing) and PROVE
// user-path reachability (a crate-index landing whose
// `crate_root.join(file)` falls inside the project root).

/// The gate's direct probe: a dirty `Accurate` project buffer holds the key,
/// the external landing lands on it — the edits, the mode, the editability
/// and the project ownership survive; the landing still makes the buffer
/// current. Content-wise it is a NO-OP (the dirty buffer is never replaced;
/// its conflict, if any, belongs to the watcher / `open_project_path`
/// machinery, not to a re-landing).
#[test]
fn external_landing_on_dirty_project_buffer_keeps_edits_and_mode() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
    let abs = dir.path().join("src/t.rs");
    std::fs::write(&abs, "fn old() {}\n").unwrap();
    let mut s = store(dir.path());
    s.open_path("src/t.rs");
    let bufk = s.buffers.current().unwrap().to_string();
    // Into Accurate mode with unsaved edits (the data-loss setup).
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    s.insert_text("zz");
    let buf = s.buffers.get(&bufk).unwrap();
    assert!(buf.locally_modified(), "precondition: dirty");
    assert_eq!(buf.mode, BufferMode::Accurate, "precondition (msg: {})", s.message);
    // The external landing lands on the key the project buffer holds.
    let landed = s.open_external_path(&abs).expect("the landing returns the existing key");
    assert_eq!(landed, bufk, "the same key (the absolute path)");
    assert!(
        s.buffer_text().contains("zz"),
        "the unsaved edit survives the landing: {:?}",
        s.buffer_text()
    );
    let buf = s.buffers.get(&bufk).unwrap();
    assert!(buf.locally_modified(), "the dirty flag survives");
    assert_eq!(
        buf.mode,
        BufferMode::Accurate,
        "the mode survives (pre-fix: the rebuild reset it to Annotation)"
    );
    assert!(buf.editable, "editability survives (pre-fix: flipped read-only)");
    assert!(
        !s.external_buffers.contains(&bufk),
        "a project buffer must not be registered as an external cache entry"
    );
    assert_eq!(s.buffers.current().unwrap(), bufk.as_str(), "the landing still makes the buffer current");
    assert!(!s.reload_confirm_active(), "no confirm: a re-landing does not discard");
}

/// REACHABILITY, proven: a project root NESTED INSIDE an indexed external
/// crate root (a project opened inside a crate source tree the tooling
/// resolver has indexed — the registry checkout / vendored-source shape the
/// sibling tooling route anticipates with its in-project `strip_prefix`
/// branch). A cross-file M-. inside the EXTERNAL buffer opens the
/// crate-rooted Xref picker; RET on the index row takes the
/// `Some(crate_root)` arm — `open_external_path(crate_root.join(file))`,
/// where the join lands INSIDE the project root, on the open, DIRTY
/// project buffer's key. Pre-fix, that RET replaced the buffer wholesale.
#[test]
fn crate_index_landing_inside_project_root_keeps_dirty_buffer() {
    let root = tempfile::tempdir().unwrap(); // the crate root R
    let sub = root.path().join("sub"); // the project root, INSIDE R
    std::fs::create_dir_all(sub.join("src")).unwrap();
    std::fs::write(sub.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(sub.join("src/app.rs"), "pub fn target() {}\n").unwrap();
    // A crate file OUTSIDE the project (the M-. origin buffer).
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    let lib = root.path().join("src/lib.rs");
    std::fs::write(&lib, "pub fn caller() {\n    target();\n}\n").unwrap();
    let mut s = store(&sub);
    // The project file is open, Accurate, and dirty.
    s.open_path("src/app.rs");
    let bufk = s.buffers.current().unwrap().to_string();
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    s.insert_text("zz");
    assert!(s.buffers.get(&bufk).unwrap().locally_modified(), "precondition: dirty");
    // The crate root's index lands (the 006-03 start_crate_indexing event).
    s.apply_crate_index_event(&CrateIndexEvent {
        source_root: root.path().to_path_buf(),
        index: build_index(
            root.path(),
            &AppStore::crate_source_files(root.path(), &["rs"]),
            None,
        ),
    });
    // Land in the EXTERNAL (crate) file, then M-. on the cross-file symbol.
    s.open_external_path(&lib).unwrap();
    s.set_point(1, 4, 4); // line 1: `    target();`
    s.xref_find_definitions();
    assert!(
        s.picker_open(),
        "a cross-file unique candidate goes to the picker (msg: {})",
        s.message
    );
    assert!(
        s.picker_filtered()[0].0.name.starts_with("sub/src/app.rs:"),
        "the candidate is crate-relative and joins INSIDE the project root: {:?}",
        s.picker_filtered()[0].0.name
    );
    // RET: the `Some(crate_root)` arm — `open_external_path(R.join("sub/src/app.rs"))`.
    s.run_selected();
    assert_eq!(
        s.buffers.current().unwrap(),
        bufk.as_str(),
        "the landing returns to the project buffer (msg: {})",
        s.message
    );
    let buf = s.buffers.get(&bufk).unwrap();
    assert!(
        s.buffer_text().contains("zz"),
        "the unsaved edit survives the crate-index landing: {:?}",
        s.buffer_text()
    );
    assert!(buf.locally_modified(), "the dirty flag survives");
    assert_eq!(buf.mode, BufferMode::Accurate, "the mode survives");
    assert!(buf.editable, "editability survives");
    assert!(
        !s.external_buffers.contains(&bufk),
        "the project buffer is not an external cache entry"
    );
}

/// The pre-fix replace had one legitimate job: refreshing a READ-ONLY
/// external cache whose file changed on disk. The guard keeps it — via
/// `reload_in_place` (content only; identity and the external registration
/// survive).
#[test]
fn external_reland_refreshes_clean_cache_in_place() {
    let (_dir, ext, mut s) = store_with_external_buffer();
    let abs = ext.path().join("registry_src.rs");
    let bufk = s.buffers.current().unwrap().to_string();
    assert!(s.external_buffers.contains(&bufk), "precondition");
    // The file changes on disk (content AND mtime — a second-granularity
    // filesystem can stamp both writes with the same mtime, so force the
    // divergence the re-stat branch compares).
    std::fs::write(&abs, "pub fn spawn2<F>(f: F) {}\n").unwrap();
    let disk = std::fs::metadata(&abs).unwrap().modified().unwrap();
    if s.buffers.get(&bufk).unwrap().mtime == disk {
        s.buffers.get_mut(&bufk).unwrap().mtime = std::time::SystemTime::UNIX_EPOCH;
    }
    s.open_external_path(&abs).unwrap();
    assert!(
        s.buffer_text().contains("spawn2"),
        "the clean cache refreshes from disk: {:?}",
        s.buffer_text()
    );
    assert_eq!(s.buffers.get(&bufk).unwrap().mtime, disk, "the recorded mtime advances");
    assert!(s.external_buffers.contains(&bufk), "still the external cache entry");
    assert!(!s.buffers.get(&bufk).unwrap().editable, "still read-only");
    assert_eq!(s.buffers.len(), 1, "no second buffer was created");
}

/// A CLEAN project buffer in `Accurate` (editability is part of the buffer's
/// identity): the landing refreshes the content IN PLACE when the on-disk
/// mtime moved — the mode and the editability survive (pre-fix the rebuild
/// reset the mode to `Annotation` and flipped it read-only).
#[test]
fn external_landing_on_clean_accurate_project_buffer_refreshes_in_place() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
    let abs = dir.path().join("src/t.rs");
    std::fs::write(&abs, "fn old() {}\n").unwrap();
    let mut s = store(dir.path());
    s.open_path("src/t.rs");
    let bufk = s.buffers.current().unwrap().to_string();
    s.key_event(key("C-x"));
    s.key_event(key("C-q"));
    assert!(
        !s.buffers.get(&bufk).unwrap().locally_modified(),
        "precondition: Accurate but clean (msg: {})",
        s.message
    );
    std::fs::write(&abs, "fn fresh() {}\n").unwrap();
    let disk = std::fs::metadata(&abs).unwrap().modified().unwrap();
    if s.buffers.get(&bufk).unwrap().mtime == disk {
        s.buffers.get_mut(&bufk).unwrap().mtime = std::time::SystemTime::UNIX_EPOCH;
    }
    s.open_external_path(&abs).unwrap();
    assert!(
        s.buffer_text().contains("fn fresh()"),
        "the clean buffer picks up the new content in place: {:?}",
        s.buffer_text()
    );
    assert_eq!(s.buffer_mode_display(), "Accurate", "the mode survives the refresh");
    assert!(s.buffers.get(&bufk).unwrap().editable, "editability survives the refresh");
    assert!(!s.buffers.get(&bufk).unwrap().locally_modified());
    assert!(!s.external_buffers.contains(&bufk), "not registered as an external entry");
}
