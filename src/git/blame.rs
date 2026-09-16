//! Blame wrapper (issue 08). git2 types stay in this module; the store
//! consumes the plain per-line `BlameLine` structs.

use std::path::Path;

use crate::git::error::GitError;
use crate::git::repo::GitRepo;

/// One blamed line: its (1-based) number, the commit that last touched it,
/// the author, the committer time, and the line text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub line_no: usize,
    pub short_id: String,
    pub author: String,
    pub time: i64,
    pub text: String,
}

impl GitRepo {
    /// Per-line blame of the workdir file at `path` (repo-relative).
    /// Blames the actual workdir content (via `blame_buffer`), so lines that
    /// are uncommitted are attributed to a zero commit id (short "0000000",
    /// author "Not Committed Yet"). This matches `git blame` / magit semantics:
    /// uncommitted changes are shown in-place and the rest of the file is
    /// correctly attributed.
    pub fn blame(&self, path: &str) -> Result<Vec<BlameLine>, GitError> {
        let mut opts = git2::BlameOptions::new();
        opts.track_copies_same_file(true);
        let blame = self.inner.blame_file(Path::new(path), Some(&mut opts))?;
        let workdir = self
            .inner
            .workdir()
            .ok_or_else(|| GitError::ReadFile {
                path: path.to_string(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "no workdir"),
            })?;
        let content = std::fs::read_to_string(workdir.join(path)).map_err(|e| GitError::ReadFile {
            path: path.to_string(),
            source: e,
        })?;
        // Blame the actual workdir bytes so line numbers match what the user
        // sees. Uncommitted lines (added/modified since HEAD) get a zero
        // final_commit_id from libgit2.
        let buf_blame = blame.blame_buffer(content.as_bytes())?;
        let zero_oid = git2::Oid::ZERO_SHA1;
        let mut out = Vec::new();
        for (i, line) in content.split_inclusive('\n').enumerate() {
            let line_no = i + 1;
            let Some(hunk) = buf_blame.get_line(line_no) else {
                continue;
            };
            let final_id = hunk.final_commit_id();
            let (short_id, author, time) = if final_id == zero_oid {
                ("0000000".to_string(), "Not Committed Yet".to_string(), 0i64)
            } else {
                let sid = final_id.to_string().chars().take(7).collect();
                let (author, time) = hunk
                    .final_signature()
                    .map(|s| (s.name().unwrap_or_default().to_string(), s.when().seconds()))
                    .unwrap_or_else(|| (String::new(), 0));
                (sid, author, time)
            };
            let text = line.strip_suffix('\n').unwrap_or(line).to_string();
            out.push(BlameLine {
                line_no,
                short_id,
                author,
                time,
                text,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn init_repo(dir: &Path) -> GitRepo {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.name", "Test"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
        GitRepo::discover(dir).expect("discover the repo")
    }

    #[test]
    fn blame_matches_git_blame_porcelain() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);

        // Build a file over three commits, touching different lines.
        std::fs::write(root.join("data.txt"), "l1\nl2\nl3\n").unwrap();
        git(root, &["add", "data.txt"]);
        git(root, &["commit", "-q", "-m", "c1"]);
        // Amend line 2 and line 3.
        std::fs::write(root.join("data.txt"), "l1\nL2\nl3\n").unwrap();
        git(root, &["add", "data.txt"]);
        git(root, &["commit", "-q", "-m", "c2"]);
        // Amend line 1.
        std::fs::write(root.join("data.txt"), "L1\nL2\nl3\n").unwrap();
        git(root, &["add", "data.txt"]);
        git(root, &["commit", "-q", "-m", "c3"]);

        let lines = g.blame("data.txt").unwrap();
        assert_eq!(lines.len(), 3, "blame: {lines:#?}");
        assert_eq!(
            lines.iter().map(|l| l.line_no).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        // The line text is exact.
        assert_eq!(lines[0].text, "L1");
        assert_eq!(lines[1].text, "L2");
        assert_eq!(lines[2].text, "l3");

        // Cross-check the per-line commit short-ids against
        // `git blame --porcelain`: each hunk's first line carries the 40-hex
        // commit id at the start of the line.
        let porcelain = git(root, &["blame", "--porcelain", "data.txt"]);
        let mut git_line_to_short: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        for l in porcelain.lines() {
            // "<sha> <orig> <final> <count>" — field 3 (1-based) is the final
            // line number.
            let parts: Vec<&str> = l.splitn(4, ' ').collect();
            if parts.len() >= 4 {
                let sha = parts[0];
                let final_line: usize = parts[2].parse().unwrap_or(0);
                if !sha.is_empty() && final_line > 0 {
                    git_line_to_short.insert(final_line, sha[..7].to_string());
                }
            }
        }
        for l in &lines {
            let expected = git_line_to_short
                .get(&l.line_no)
                .expect("git blame --porcelain should cover every line");
            assert_eq!(
                &l.short_id, expected,
                "line {} blame short id mismatch",
                l.line_no
            );
        }
        // The author is the configured identity on every line.
        assert!(lines.iter().all(|l| l.author == "Test"));
    }

    #[test]
    fn blame_dirty_file_matches_git_blame_porcelain() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);

        // Build a file over two commits.
        std::fs::write(root.join("data.txt"), "l1\nl2\nl3\nl4\n").unwrap();
        git(root, &["add", "data.txt"]);
        git(root, &["commit", "-q", "-m", "c1"]);
        std::fs::write(root.join("data.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        git(root, &["add", "data.txt"]);
        git(root, &["commit", "-q", "-m", "c2"]);

        // Make the workdir dirty: insert a new line after line 2, and modify
        // line 4. This shifts HEAD-coordinate line numbers.
        std::fs::write(
            root.join("data.txt"),
            "l1\nl2\nNEW\nl4-modified\nl5\n",
        )
        .unwrap();

        let lines = g.blame("data.txt").unwrap();
        assert_eq!(lines.len(), 5, "blame: {lines:#?}");

        // Line 3 is "NEW" (uncommitted) → zero commit, "Not Committed Yet".
        assert_eq!(lines[2].text, "NEW");
        assert_eq!(lines[2].short_id, "0000000");
        assert_eq!(lines[2].author, "Not Committed Yet");

        // Line 4 is "l4-modified" (uncommitted) → zero commit.
        assert_eq!(lines[3].text, "l4-modified");
        assert_eq!(lines[3].short_id, "0000000");
        assert_eq!(lines[3].author, "Not Committed Yet");

        // Lines 1, 2, 5 should match their committed blame (c1 or c2).
        // Cross-check against git blame --porcelain on the workdir file.
        // git blame on a dirty file shows uncommitted lines with 0000000…
        let porcelain = git(root, &["blame", "--porcelain", "data.txt"]);
        let mut git_line_to_short: std::collections::HashMap<usize, String> =
            std::collections::HashMap::new();
        for l in porcelain.lines() {
            // "<sha> <orig-line> <final-line> [num-lines]" — field 3 (1-based)
            // is the final line number.
            let parts: Vec<&str> = l.splitn(4, ' ').collect();
            if parts.len() >= 3 && parts[0].len() >= 40 {
                let sha = parts[0];
                let final_line: usize = parts[2].parse().unwrap_or(0);
                if final_line > 0 {
                    git_line_to_short.insert(final_line, sha[..7].to_string());
                }
            }
        }
        for l in &lines {
            let expected = git_line_to_short
                .get(&l.line_no)
                .expect("git blame --porcelain should cover every line");
            assert_eq!(
                &l.short_id, expected,
                "line {} blame short id mismatch",
                l.line_no
            );
        }
    }

    #[test]
    fn blame_missing_file_is_read_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        // A file that is in neither the workdir nor the index cannot be
        // blamed; the call must fail cleanly (not panic).
        assert!(g.blame("nope.txt").is_err());
    }
}
