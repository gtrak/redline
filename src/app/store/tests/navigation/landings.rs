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
        AppStore::first_word_column("pub fn spawn() {}", "spawn"),
        Some(7)
    );
    // Whole-word only: `run` inside `running` is NOT a match.
    assert_eq!(
        AppStore::first_word_column("pub fn running() {}", "run"),
        None,
        "a leading word-char disqualifies"
    );
    // A trailing word-char also disqualifies (`respawn`).
    assert_eq!(AppStore::first_word_column("respawn()", "spawn"), None);
    // Multibyte prefix: `café spawn` — `spawn` at CHAR 5, not byte 6.
    assert_eq!(
        AppStore::first_word_column("café spawn()", "spawn"),
        Some(5),
        "a char column, not a byte offset"
    );
    // Absent entirely.
    assert_eq!(
        AppStore::first_word_column("pub fn spawn() {}", "target"),
        None
    );
    // Empty item: no match (never a spurious col 0 hit).
    assert_eq!(AppStore::first_word_column("spawn()", ""), None);
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
