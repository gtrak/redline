//! Tests for the git repo wrapper, moved verbatim from the old
//! `repo.rs`. Kept as a child of the `repo` module (via `mod tests;` in
//! `mod.rs`) because the tests read `GitRepo`'s private state.

use super::*;
use crate::git::diff::DiffOrigin;
use crate::git::status::Side;
use crate::model::sections::StatusTree;
use std::collections::HashMap;
use std::path::Path;

/// Run the git CLI in `dir`, with an isolated global config and
/// committed author/committer identity so the tests are hermetic.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Back-date `path`'s mtime/ctime 5 s into the past (GNU `touch`). This puts
/// the workdir file's stat outside git's racy-timestamp window, so a stat
/// cache that "matches" is trusted and the re-hash that would expose a lie is
/// skipped. The reported stat-cache-lie reproduction forces exactly this.
fn backdate(path: &Path) {
    let out = std::process::Command::new("touch")
        .args(["-d", "5 seconds ago"])
        .arg(path)
        .output()
        .expect("run touch");
    assert!(
        out.status.success(),
        "touch -d failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) -> GitRepo {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    GitRepo::discover(dir).expect("discover the repo")
}

/// Build a `StatusTree` the same way the store does (status + per-file
/// diffs).
fn build_tree(g: &GitRepo) -> StatusTree {
    let status = g.status().unwrap();
    let mut staged = HashMap::new();
    let mut unstaged = HashMap::new();
    for f in &status.files {
        if f.is_staged() && let Ok(d) = g.diff(DiffSide::Staged, &f.path) {
            staged.insert(f.path.clone(), d);
        }
        if f.is_unstaged() && let Ok(d) = g.diff(DiffSide::Unstaged, &f.path) {
            unstaged.insert(f.path.clone(), d);
        }
    }
    StatusTree::build(&status, &staged, &unstaged, None)
}

#[test]
fn status_classification_matches_git_cli() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);

    // Baseline: a.txt, e.txt, and f.txt are committed.
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    std::fs::write(root.join("e.txt"), "e\n").unwrap();
    std::fs::write(root.join("f.txt"), "f\n").unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["add", "e.txt"]);
    git(root, &["add", "f.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // a.txt: unstaged modification only
    std::fs::write(root.join("a.txt"), "A\n").unwrap();
    // c.txt: staged addition
    std::fs::write(root.join("c.txt"), "c\n").unwrap();
    git(root, &["add", "c.txt"]);
    // d.txt: untracked
    std::fs::write(root.join("d.txt"), "d\n").unwrap();
    // staged rename: HEAD has e.txt, index has e2.txt
    git(root, &["mv", "e.txt", "e2.txt"]);
    // f.txt: staged AND unstaged
    std::fs::write(root.join("f.txt"), "F1\n").unwrap();
    git(root, &["add", "f.txt"]);
    std::fs::write(root.join("f.txt"), "F2\n").unwrap();

    let status = g.status().unwrap();

    // Cross-check the number of changed entries against the git CLI
    // (rename detection forced on so a rename is one entry, as in the
    // wrapper).
    let porcelain = git(root, &["-c", "status.renames=true", "status", "--porcelain"]);
    let porcelain_lines: Vec<&str> = porcelain.lines().collect();
    assert_eq!(
        porcelain_lines.len(),
        status.files.len(),
        "porcelain:\n{porcelain}\nwrapper:\n{status:#?}"
    );

    let find = |p: &str| -> &FileStatus {
        status
            .files
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| panic!("missing {p} in {status:#?}"))
    };
    assert_eq!(find("a.txt").unstaged, StatusKind::Modified);
    assert_eq!(find("a.txt").staged, StatusKind::None);
    assert_eq!(find("c.txt").staged, StatusKind::Added);
    assert_eq!(find("c.txt").unstaged, StatusKind::None);
    assert!(find("d.txt").untracked);
    assert_eq!(find("e2.txt").staged, StatusKind::Renamed);
    assert_eq!(find("e2.txt").orig, Some("e.txt".into()));
    assert_eq!(find("f.txt").staged, StatusKind::Modified);
    assert_eq!(find("f.txt").unstaged, StatusKind::Modified);

    assert_eq!(status.staged_count(), 3);   // c, e2, f
    assert_eq!(status.unstaged_count(), 2); // a, f
    assert_eq!(status.untracked_count(), 1); // d
}

#[test]
fn stage_file_then_unstage_matches_cli() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("a.txt"), "a\nb\nc\n").unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    std::fs::write(root.join("a.txt"), "a\nB\nc\n").unwrap();
    g.stage_file("a.txt").unwrap();

    // wrapper staged diff agrees with `git diff --cached`
    let staged = g.diff(DiffSide::Staged, "a.txt").unwrap();
    assert_eq!(staged.hunks.len(), 1);
    assert!(staged.hunks[0]
        .lines
        .iter()
        .any(|l| l.content == "B" && l.origin == DiffOrigin::Addition));
    assert!(staged.hunks[0]
        .lines
        .iter()
        .any(|l| l.content == "b" && l.origin == DiffOrigin::Deletion));
    let cached = git(root, &["diff", "--cached"]);
    assert!(cached.contains("+B"), "cached:\n{cached}");
    assert!(cached.contains("-b"), "cached:\n{cached}");
    // workdir now equals index: unstaged diff empty
    assert!(git(root, &["diff"]).trim().is_empty());

    // unstage: index reverts to HEAD, workdir keeps the change
    g.unstage_file("a.txt", None).unwrap();
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "after unstage, --cached must be empty"
    );
    assert!(git(root, &["diff"]).contains("+B"));
}

#[test]
fn unstage_file_stat_cache_lie_loop() {
    // Regression for issue-git-stat-cache-lie: `unstage_file` must not leave
    // the workdir file's stat cache on an index entry that names HEAD's blob.
    // If it does, `git status` sees a matching stat, git's racy-timestamp
    // re-hash never fires, and a genuinely modified file is reported clean.
    //
    // This is a LOOP, on purpose. A single iteration passes even with the bug
    // present, because the lie only surfaces when the workdir mtime is >= 2s
    // old (git's racy window). `backdate` forces that, making every iteration
    // a deterministic failure on the unfixed code (the reported reproduction:
    // ~1/1200 naturally, 200/200 when forced).
    const ITERS: u32 = 25;
    for i in 1..=ITERS {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("a.txt"), "a\nb\nc\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // The change, on disk: HEAD is "a\nb\nc\n", workdir is "a\nB\nc\n".
        std::fs::write(root.join("a.txt"), "a\nB\nc\n").unwrap();
        // Force the mtime into the past so the racy re-hash that would mask
        // the lie on a fresh mtime does not fire.
        backdate(&root.join("a.txt"));

        g.stage_file("a.txt").unwrap();
        g.unstage_file("a.txt", None).unwrap();

        // The workdir file is modified relative to HEAD. The CLI (the oracle)
        // must report it as unstaged-modified; the wrapper must agree.
        let porcelain = git(root, &["status", "--porcelain"]);
        let cli_modified = porcelain
            .lines()
            .any(|l| l.starts_with(" M") && l.contains("a.txt"));
        let st = g.status().unwrap();
        let wrapper_modified = st
            .files
            .iter()
            .any(|f| f.path == "a.txt" && f.unstaged == StatusKind::Modified);

        assert!(
            cli_modified && wrapper_modified,
            "iteration {i}: an unstage left a modified file reading clean\
             (stat-cache lie). CLI and wrapper must both report a.txt\
             modified, but CLI_modified={cli_modified}\
             wrapper_modified={wrapper_modified}\n\
             porcelain={porcelain:?}\n\
             workdir={:?}",
            std::fs::read_to_string(root.join("a.txt")).unwrap()
        );

        // Reverse direction stays correct: `git diff --cached` (index vs HEAD)
        // must be empty after an unstage — the index named HEAD's blob.
        assert!(
            git(root, &["diff", "--cached"]).trim().is_empty(),
            "iteration {i}: --cached must be empty after unstage"
        );

        // Clean-shown-clean: restore the workdir to HEAD. The zeroed stat
        // must NOT turn a now-matching file into a false "modified".
        std::fs::write(root.join("a.txt"), "a\nb\nc\n").unwrap();
        backdate(&root.join("a.txt"));
        let clean_porcelain = git(root, &["status", "--porcelain"]);
        assert!(
            !clean_porcelain.lines().any(|l| l.contains("a.txt")),
            "iteration {i}: a clean file is shown modified (false positive):\n{clean_porcelain}"
        );
    }
}

#[test]
fn stage_middle_hunk_stages_only_that_hunk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Three far-apart edits → three hunks.
    let mut w = base.clone();
    w[1] = "MOD-2".into();
    w[14] = "MOD-15".into();
    w[27] = "MOD-28".into();
    std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();

    let d = g.diff(DiffSide::Unstaged, "big.txt").unwrap();
    assert_eq!(d.hunks.len(), 3, "hunks:\n{d:#?}");
    let mid = d
        .hunks
        .iter()
        .find(|h| h.lines.iter().any(|l| l.content == "MOD-15"))
        .unwrap();
    g.stage_hunk("big.txt", mid.new_start).unwrap();

    // The index gained exactly the middle hunk.
    let cached = git(root, &["diff", "--cached"]);
    assert!(cached.contains("MOD-15"), "cached:\n{cached}");
    assert!(!cached.contains("MOD-28"), "cached leaked MOD-28:\n{cached}");
    assert!(!cached.contains("MOD-2\n"), "cached leaked MOD-2:\n{cached}");
    let staged = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(staged.hunks.len(), 1, "staged:\n{staged:#?}");
    assert!(staged.hunks[0].lines.iter().any(|l| l.content == "MOD-15"));
    // The workdir still carries all three edits (index-only staging).
    let workdir = git(root, &["diff"]);
    assert!(
        workdir.contains("MOD-2\n") && workdir.contains("MOD-28"),
        "workdir:\n{workdir}"
    );
}

#[test]
fn unstage_middle_hunk_reverts_index_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    let mut w = base.clone();
    w[1] = "MOD-2".into();
    w[14] = "MOD-15".into();
    w[27] = "MOD-28".into();
    std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();
    g.stage_file("big.txt").unwrap(); // stage all three

    let staged = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(staged.hunks.len(), 3);
    let mid = staged
        .hunks
        .iter()
        .find(|h| h.lines.iter().any(|l| l.content == "MOD-15"))
        .unwrap();
    g.unstage_hunk("big.txt", mid.new_start).unwrap();

    // The index lost the middle hunk; the other two remain staged.
    let staged2 = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(staged2.hunks.len(), 2, "staged after unstage:\n{staged2:#?}");
    assert!(!staged2
        .hunks
        .iter()
        .any(|h| h.lines.iter().any(|l| l.content == "MOD-15")));
    let cached = git(root, &["diff", "--cached"]);
    assert!(!cached.contains("MOD-15"), "cached:\n{cached}");
    assert!(cached.contains("MOD-28"), "cached:\n{cached}");
    assert!(cached.contains("MOD-2\n"), "cached must keep hunk1:\n{cached}");
    // The workdir still carries the middle edit.
    assert!(git(root, &["diff"]).contains("MOD-15"));
}

#[test]
fn unstage_hunk_on_no_trailing_newline_file_keeps_index_exact() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    // 30 lines; the last line (line30) has NO trailing newline.
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
    let base_text: String = base[..29].join("\n") + "\n" + base[29].as_str();
    std::fs::write(root.join("big.txt"), &base_text).unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Three far-apart edits; the last touches the file's final line.
    let mut w = base.clone();
    w[1] = "MOD-2".into();
    w[14] = "MOD-15".into();
    w[29] = "MOD-30".into();
    let w_text: String = w[..29].join("\n") + "\n" + w[29].as_str();
    std::fs::write(root.join("big.txt"), &w_text).unwrap();

    let d = g.diff(DiffSide::Unstaged, "big.txt").unwrap();
    assert_eq!(d.hunks.len(), 3, "hunks:\n{d:#?}");
    g.stage_file("big.txt").unwrap(); // stage all three

    // Unstage the hunk that touches the old file's final line.
    let staged = g.diff(DiffSide::Staged, "big.txt").unwrap();
    let last = staged
        .hunks
        .iter()
        .find(|h| h.lines.iter().any(|l| l.content == "MOD-30"))
        .expect("the final-line hunk must be staged");
    // Marker note: xdiff keys the EOFNL marker to the last hunk line's
    // prefix ('+' here -> DeleteEOFNL) even when BOTH sides lack the
    // trailing LF, so the marker-derived flag is not the semantic truth
    // -- the byte-exact assertions below are the real contract.
    assert!(last.old_ends_nl, "DeleteEOFNL marker maps old_ends_nl=true");
    g.unstage_hunk("big.txt", last.new_start).unwrap();

    // The index blob must equal HEAD byte-for-byte except for the two
    // remaining hunks — including the final line's missing newline.
    let staged_blob = git(root, &["show", ":big.txt"]);
    let mut expected = base.clone();
    expected[1] = "MOD-2".into();
    expected[14] = "MOD-15".into();
    let expected_text: String =
        expected[..29].join("\n") + "\n" + expected[29].as_str();
    assert_eq!(
        staged_blob, expected_text,
        "index blob must be byte-exact"
    );

    // `git diff --cached` shows the remaining hunks and nothing else:
    // no MOD-30, and no leaked EOFNL marker text.
    let cached = git(root, &["diff", "--cached"]);
    assert!(cached.contains("MOD-2"), "cached:\n{cached}");
    assert!(cached.contains("MOD-15"), "cached:\n{cached}");
    assert!(!cached.contains("MOD-30"), "cached:\n{cached}");
    assert!(!cached.contains("No newline"), "marker leaked into index:\n{cached}");

    // The wrapper agrees: exactly the two remaining hunks, staged side.
    let staged2 = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(staged2.hunks.len(), 2, "staged:\n{staged2:#?}");
    assert!(!staged2
        .hunks
        .iter()
        .any(|h| h.lines.iter().any(|l| l.content == "MOD-30")));
    // The workdir still carries the final-line edit (now unstaged).
    let workdir = git(root, &["diff"]);
    assert!(workdir.contains("MOD-30"), "workdir:\n{workdir}");
    // MOD-2 / MOD-15 are still staged: they do NOT appear in the
    // index-vs-workdir diff.
    assert!(!workdir.contains("MOD-15"), "workdir:\n{workdir}");
}

#[test]
fn unstage_hunk_refuses_non_utf8_content() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    // A "text" file with an invalid UTF-8 byte (0xFF, no NULs): libgit2
    // still produces text hunks for it.
    std::fs::write(root.join("latin.txt"), b"a\xff\nb\n").unwrap();
    git(root, &["add", "latin.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);
    std::fs::write(root.join("latin.txt"), b"a\xff\nB\n").unwrap();
    g.stage_file("latin.txt").unwrap();

    let staged = g.diff(DiffSide::Staged, "latin.txt").unwrap();
    assert!(
        !staged.hunks.is_empty(),
        "non-UTF-8 text file must produce hunks"
    );
    let err = g
        .unstage_hunk("latin.txt", staged.hunks[0].new_start)
        .unwrap_err();
    assert!(
        matches!(err, GitError::NotUtf8(ref p) if p == "latin.txt"),
        "unexpected error: {err:?}"
    );
    // The index is untouched: the hunk is still staged.
    assert!(git(root, &["diff", "--cached"]).contains("+B"));
}

#[test]
fn stage_deleted_file_stages_the_removal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("gone.txt"), "content\n").unwrap();
    git(root, &["add", "gone.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Delete from workdir, then stage: the index entry must be removed.
    std::fs::remove_file(root.join("gone.txt")).unwrap();
    g.stage_file("gone.txt").unwrap();

    // `git diff --cached` must show the deletion.
    let cached = git(root, &["diff", "--cached"]);
    assert!(cached.contains("-content"), "cached:\n{cached}");
    // `git status --porcelain` must show it as staged-deleted ("D  ").
    let porcelain = git(root, &["status", "--porcelain"]);
    assert!(
        porcelain.starts_with("D ") || porcelain.starts_with("D\t"),
        "porcelain:\n{porcelain}"
    );
    // The unstaged diff must be empty (workdir matches index: both absent).
    assert!(git(root, &["diff"]).trim().is_empty());
}

#[test]
fn staged_rename_reported_with_orig_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("doc.txt"), "hello world\n").unwrap();
    git(root, &["add", "doc.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    git(root, &["mv", "doc.txt", "docs.txt"]);
    let status = g.status().unwrap();
    let entry = status
        .files
        .iter()
        .find(|f| f.path == "docs.txt")
        .expect("renamed entry must be present");
    assert_eq!(entry.staged, StatusKind::Renamed);
    assert_eq!(entry.orig, Some("doc.txt".into()));
    assert!(!status.files.iter().any(|f| f.path == "doc.txt"));
}

#[test]
fn discard_unstaged_file_restores_workdir_to_head() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("a.txt"), "keep\n").unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Unstaged modification only.
    std::fs::write(root.join("a.txt"), "changed\n").unwrap();
    g.discard_unstaged_file("a.txt").unwrap();

    // Workdir restored to HEAD; index unchanged (still matches HEAD).
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "keep\n"
    );
    assert!(
        git(root, &["diff"]).trim().is_empty(),
        "workdir must match HEAD after discard"
    );
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "index must be untouched"
    );
}

#[test]
fn discard_staged_file_clears_index_and_workdir() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("a.txt"), "keep\n").unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Staged modification.
    std::fs::write(root.join("a.txt"), "staged\n").unwrap();
    git(root, &["add", "a.txt"]);
    assert!(git(root, &["diff", "--cached"]).contains("+staged"));

    g.discard_staged_file("a.txt", None).unwrap();

    // Both index and workdir restored to HEAD.
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "keep\n"
    );
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "index must match HEAD"
    );
    assert!(
        git(root, &["diff"]).trim().is_empty(),
        "workdir must match HEAD"
    );
}

#[test]
fn discard_staged_addition_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("base.txt"), "base\n").unwrap();
    git(root, &["add", "base.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Stage a brand-new file.
    std::fs::write(root.join("new.txt"), "added\n").unwrap();
    git(root, &["add", "new.txt"]);
    assert!(git(root, &["diff", "--cached"]).contains("+added"));

    g.discard_staged_file("new.txt", None).unwrap();

    // The index entry is gone and the workdir file is removed.
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "staged addition must be dropped"
    );
    assert!(!root.join("new.txt").exists(), "workdir file must be removed");
}

#[test]
fn discard_unstaged_hunk_reverts_only_that_hunk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Two far-apart edits → two hunks (both unstaged).
    let mut w = base.clone();
    w[1] = "MOD-2".into();
    w[20] = "MOD-21".into();
    std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();

    let d = g.diff(DiffSide::Unstaged, "big.txt").unwrap();
    assert_eq!(d.hunks.len(), 2, "hunks:\n{d:#?}");
    let second = d
        .hunks
        .iter()
        .find(|h| h.lines.iter().any(|l| l.content == "MOD-21"))
        .unwrap();
    g.discard_hunk("big.txt", Some(Side::Unstaged), second.new_start)
        .unwrap();

    // Only the target hunk reverted in the workdir; the other survives.
    let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
    assert!(
        after.contains("line21"),
        "the discarded hunk's line must be restored: {after}"
    );
    assert!(
        !after.contains("MOD-21"),
        "the discarded change must be gone: {after}"
    );
    assert!(
        after.contains("MOD-2"),
        "the untouched hunk must survive: {after}"
    );
    // The index is untouched (nothing was staged).
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "index must be untouched"
    );
}

#[test]
fn discard_staged_hunk_reverts_workdir_and_unstages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")) .collect();
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Two far-apart edits: stage the first, leave the second unstaged.
    let mut w = base.clone();
    w[1] = "STAGED-MOD".into(); // hunk 1: staged
    w[20] = "UNSTAGED-MOD".into(); // hunk 2: unstaged
    std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();

    // Stage only the first hunk by committing the full workdir, then
    // re-write to get a clean state with staged + unstaged mixed.
    // Simpler: write both mods, stage the file, then add the second mod.
    // Actually the simplest is: write both mods, stage the file (both
    // staged), then add a third change (unstaged). But we want to test
    // the staged hunk path specifically.
    //
    // Cleanest approach: write the first mod, stage it. Write the second
    // mod without staging. Now hunk 1 is staged, hunk 2 is unstaged.
    // But git's diff will show them differently. Let's use a simpler
    // scenario: one staged change and one unstaged change in the same file.
    //
    // Reset: re-initialize.
    // Actually let me just do it in one shot: write both mods, stage the
    // file (both staged), then modify one line back to create an
    // unstaged delta. No, that gets complicated.
    //
    // Simplest: just test the pure staged case first (all changes staged),
    // then verify the workdir + index are correct.
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    // Now make one change and stage it.
    let mut staged_only = base.clone();
    staged_only[1] = "STAGED-MOD".into();
    std::fs::write(root.join("big.txt"), staged_only.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    assert!(
        git(root, &["diff", "--cached"]).contains("STAGED-MOD"),
        "staged change present in index"
    );

    // Get the staged diff and find the hunk.
    let d = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
    let hunk = &d.hunks[0];

    g.discard_hunk("big.txt", Some(Side::Staged), hunk.new_start)
        .unwrap();

    // Workdir must match HEAD (the staged change is reverted in workdir).
    let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
    assert!(
        after.contains("line02"),
        "workdir restored to HEAD: {after}"
    );
    assert!(
        !after.contains("STAGED-MOD"),
        "staged mod must be gone from workdir: {after}"
    );
    // Index must also be clean (the hunk was unstaged).
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "index must match HEAD after staged-hunk discard"
    );
    assert!(
        git(root, &["diff"]).trim().is_empty(),
        "workdir must match HEAD after staged-hunk discard"
    );
}

#[test]
fn discard_staged_hunk_with_mixed_staged_unstaged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")) .collect();
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Hunk 1 (near line 2): STAGED.  Hunk 2 (near line 21): UNSTAGED.
    // Write both changes, stage the file, then add a further unstaged
    // edit to a different region. Actually: write both, stage the file
    // (both staged). Then we need one to be unstaged.
    //
    // The cleanest way: write mod 1, stage it. Write mod 2 (now
    // workdir has both, index has only mod 1).
    let mut w = base.clone();
    w[1] = "STAGED-MOD".into();
    std::fs::write(root.join("big.txt"), w.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    // Now add the unstaged mod.
    let mut w2 = w.clone();
    w2[20] = "UNSTAGED-MOD".into();
    std::fs::write(root.join("big.txt"), w2.join("\n") + "\n").unwrap();

    // Verify: index has STAGED-MOD but not UNSTAGED-MOD.
    let cached = git(root, &["diff", "--cached"]);
    assert!(cached.contains("STAGED-MOD"), "index has staged mod");
    assert!(!cached.contains("UNSTAGED-MOD"), "index must not have unstaged mod");
    let unstaged = git(root, &["diff"]);
    assert!(unstaged.contains("UNSTAGED-MOD"), "workdir has unstaged mod");

    // Now discard the staged hunk.
    let d = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
    g.discard_hunk("big.txt", Some(Side::Staged), d.hunks[0].new_start)
        .unwrap();

    // The staged change is gone from both index and workdir.
    let cached_after = git(root, &["diff", "--cached"]);
    assert!(!cached_after.contains("STAGED-MOD"), "staged mod unstaged");
    // The unstaged change survives in the workdir.
    let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
    assert!(
        after.contains("UNSTAGED-MOD"),
        "unstaged mod must survive: {after}"
    );
    // Use exact-line match: "UNSTAGED-MOD" contains "STAGED-MOD" as a substring.
    assert!(
        !after.lines().any(|l| l == "STAGED-MOD"),
        "staged mod must be reverted from workdir: {after}"
    );
    // The unstaged diff still shows the surviving change.
    let unstaged_after = git(root, &["diff"]);
    assert!(unstaged_after.contains("UNSTAGED-MOD"), "unstaged mod still in workdir diff");
}

#[test]
fn discard_staged_hunk_with_unstaged_insertion_above() {
    // Repro for the blocking finding: an unstaged insertion above the
    // staged hunk shifts line numbers, so the workdir splice must use
    // context matching, not the index-side line offset.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    let base: Vec<String> = (1..=30).map(|i| format!("line{i:02}")).collect();
    std::fs::write(root.join("big.txt"), base.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Stage a change to line 5 (index now has "STAGED-MOD" at line 5).
    let mut staged = base.clone();
    staged[4] = "STAGED-MOD".into();
    std::fs::write(root.join("big.txt"), staged.join("\n") + "\n").unwrap();
    git(root, &["add", "big.txt"]);

    // Now insert a line at the top of the workdir (unstaged). This shifts
    // all workdir line numbers by +1 relative to the index.
    let mut shifted = vec!["INSERTED".to_string()];
    shifted.extend(staged);
    std::fs::write(root.join("big.txt"), shifted.join("\n") + "\n").unwrap();

    // Verify: the index has the staged change, the workdir has the
    // insertion + the staged change (shifted down by one line).
    let cached = git(root, &["diff", "--cached"]);
    assert!(cached.contains("STAGED-MOD"), "staged mod in index");
    let unstaged = git(root, &["diff"]);
    assert!(unstaged.contains("INSERTED"), "insertion is unstaged");

    // Get the staged hunk's new_start (index-side line number).
    let d = g.diff(DiffSide::Staged, "big.txt").unwrap();
    assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
    let hunk_start = d.hunks[0].new_start;
    // In the index, the hunk starts at line 2 (3 context lines before
    // the change at line 5). In the workdir, it starts at line 3
    // (shifted by the insertion). The old code would splice at line 2
    // (wrong); the fix must find the content by context.
    assert_eq!(hunk_start, 2, "index-side hunk start (with context)");

    g.discard_hunk("big.txt", Some(Side::Staged), hunk_start)
        .unwrap();

    // The staged change must be gone from the workdir (reverted to
    // "line05"), but the insertion must survive.
    let after = std::fs::read_to_string(root.join("big.txt")).unwrap();
    // Byte-exact assertion: the workdir must be exactly "INSERTED\n" +
    // all 30 HEAD lines. This discriminates against the old
    // index-side line splice which would lose line01 and duplicate
    // line08.
    let expected = format!("INSERTED\n{}\n", base.join("\n"));
    assert_eq!(
        after, expected,
        "workdir must be byte-exact: INSERTED + all 30 HEAD lines"
    );
    // The index must also be clean (the hunk was unstaged).
    assert!(
        git(root, &["diff", "--cached"]).trim().is_empty(),
        "index must match HEAD after staged-hunk discard"
    );
    // The workdir diff must show only the insertion (not the staged mod).
    let unstaged_after = git(root, &["diff"]);
    assert!(unstaged_after.contains("INSERTED"));
    assert!(!unstaged_after.contains("STAGED-MOD"));
}

#[test]
fn discard_untracked_file_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let _g = init_repo(root);
    std::fs::write(root.join("scratch.txt"), "untracked\n").unwrap();

    _g.discard_untracked_file("scratch.txt").unwrap();

    assert!(!root.join("scratch.txt").exists(), "file must be deleted");
}

#[test]
fn discard_staged_hunk_at_eof_no_trailing_newline() {
    // Regression test for the phantom trailing newline: when a staged
    // hunk reaches EOF of a file with no trailing newline, the
    // new-side search must NOT append a phantom \\n that isn't in the
    // workdir. Before the fix, `reverse_apply_hunk_in_content` would
    // fail with HunkNotFound because it searched for "a\nb\nB\nend\n"
    // when the workdir actually contains "a\nb\nB\nend".
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    // HEAD: "a\nb\nend" (no trailing newline).
    std::fs::write(root.join("f.txt"), "a\nb\nend").unwrap();
    git(root, &["add", "f.txt"]);
    git(root, &["commit", "-q", "-m", "init"]);

    // Stage b->B (index: "a\nB\nend", no trailing NL).
    std::fs::write(root.join("f.txt"), "a\nB\nend").unwrap();
    git(root, &["add", "f.txt"]);

    // Workdir == index (no unstaged changes).
    let workdir_before = std::fs::read(root.join("f.txt")).unwrap();
    assert_eq!(workdir_before, b"a\nB\nend");

    // Get the staged hunk.
    let d = g.diff(DiffSide::Staged, "f.txt").unwrap();
    assert_eq!(d.hunks.len(), 1, "one staged hunk: {d:#?}");
    let hunk_start = d.hunks[0].new_start;

    // Discard the staged hunk: must succeed and restore "a\\nb\\nend".
    g.discard_hunk("f.txt", Some(Side::Staged), hunk_start)
        .expect("discard must succeed for EOF-no-NL file");

    let after = std::fs::read(root.join("f.txt")).unwrap();
    assert_eq!(
        after, b"a\nb\nend",
        "workdir must be byte-exact after staged-hunk discard at EOF"
    );
}

#[test]
fn snapshot_real_repo_status_tree() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let g = init_repo(root);
    std::fs::write(root.join("app.rs"), "fn main() {}\n").unwrap();
    git(root, &["add", "app.rs"]);
    git(root, &["commit", "-q", "-m", "init"]);
    // staged change…
    std::fs::write(root.join("app.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    g.stage_file("app.rs").unwrap();
    // …and a further unstaged change, plus an untracked file.
    std::fs::write(
        root.join("app.rs"),
        "fn main() {\n    println!(\"hello\");\n    println!(\"world\");\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("scratch.txt"), "untracked\n").unwrap();

    let mut tree = build_tree(&g);
    // Open the first file so its hunk body appears in the snapshot.
    tree.move_down();
    tree.toggle_fold();
    let rows: Vec<String> = tree
        .visible_rows()
        .iter()
        .map(|r| format!("{:?} | {}", r.role, r.text))
        .collect();
    insta::assert_debug_snapshot!(rows);
}
