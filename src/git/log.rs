//! Log (revwalk) + commit-diff wrappers. git2 types stay in this module
//! (issue 08); the UI and store consume the plain `LogEntry` / `CommitDiff`
//! structs returned here.

use crate::git::diff::extract_commit;
use crate::git::error::GitError;
use crate::git::repo::GitRepo;

/// One line of a magit-style log: short hash, subject, author, and a
/// relative age. `time` is the committer unix time (seconds); `date` is a
/// human relative-age string measured from the current time at build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub short_id: String,
    pub subject: String,
    pub author: String,
    pub time: i64,
    pub date: String,
}

/// The full tree diff of one commit, as plain per-file diffs + aggregate
/// stats. This is what the log view's `RET` opens read-only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitDiff {
    pub short_id: String,
    pub subject: String,
    pub insertions: u32,
    pub deletions: u32,
    pub files: Vec<crate::git::diff::FileDiff>,
}

impl GitRepo {
    /// The number of commits reachable from `branch` (or HEAD when `None`).
    pub fn log_total(&self, branch: Option<&str>) -> Result<usize, GitError> {
        Ok(revwalk(&self.inner, branch)?.count())
    }

    /// A page of the log: the `limit` commits starting at `offset`, in
    /// newest-first order (topological + committer time). `branch` is a
    /// local branch name; `None` walks HEAD. Windows are stable for a fixed
    /// history: `offset`/`limit` are positions in the total walk.
    pub fn log(
        &self,
        branch: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<LogEntry>, GitError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let walk = revwalk(&self.inner, branch)?;
        let mut out = Vec::new();
        for (i, oid) in walk.enumerate() {
            if i < offset {
                continue;
            }
            if out.len() >= limit {
                break;
            }
            let commit = self.inner.find_commit(oid?)?;
            out.push(log_entry(&commit));
        }
        Ok(out)
    }

    /// The full tree diff of the commit at `oid` (a full or abbreviated hex
    /// id), against its first parent (or the empty tree for the root commit).
    pub fn commit_diff(&self, oid: &str) -> Result<CommitDiff, GitError> {
        let commit = self.find_commit_flex(oid)?;
        let short_id = commit.id().to_string().chars().take(7).collect();
        let subject = commit.summary()?.unwrap_or_default().to_string();
        let tree = commit.tree()?;
        let parent_tree: Option<git2::Tree> = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let diff = self
            .inner
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let stats = diff.stats()?;
        let files = extract_commit(&diff);
        Ok(CommitDiff {
            short_id,
            subject,
            insertions: stats.insertions() as u32,
            deletions: stats.deletions() as u32,
            files,
        })
    }

    /// Resolve a full or abbreviated commit id to a commit. `Oid::from_str`
    /// zero-pads abbreviations (git2 skill gotcha #15), so short hashes go
    /// through `find_commit_by_prefix`.
    fn find_commit_flex(&self, oid: &str) -> Result<git2::Commit<'_>, GitError> {
        if oid.len() >= 40 {
            return self
                .inner
                .find_commit(git2::Oid::from_str(oid)?)
                .map_err(Into::into);
        }
        self.inner
            .find_commit_by_prefix(oid)
            .map_err(Into::into)
    }
}

/// A fresh revwalk pushed to `branch` (or HEAD) with the magit log sort
/// (topological + time: newest first, topologically sound).
fn revwalk<'r>(repo: &'r git2::Repository, branch: Option<&str>) -> Result<git2::Revwalk<'r>, GitError> {
    let mut walk = repo.revwalk()?;
    match branch {
        Some(b) if !b.is_empty() => walk.push_ref(&format!("refs/heads/{b}"))?,
        _ => walk.push_head()?,
    }
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
    Ok(walk)
}

fn log_entry(commit: &git2::Commit) -> LogEntry {
    let cid = commit.id();
    let short_id = cid.to_string().chars().take(7).collect();
    let subject = commit
        .summary()
        .unwrap_or(None)
        .unwrap_or_default()
        .to_string();
    let author = commit
        .author()
        .name()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let time = commit.time().seconds();
    LogEntry {
        short_id,
        subject,
        author,
        time,
        date: relative_time(time),
    }
}

/// A human relative age for a unix time, measured from now.
pub fn relative_time(time: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    relative_time_from(time, now)
}

/// A pure relative-age (no `SystemTime`), for deterministic rendering/tests.
pub fn relative_time_from(time: i64, now: i64) -> String {
    let d = now.saturating_sub(time);
    if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else if d < 86_400 * 30 {
        format!("{}d", d / 86_400)
    } else {
        format!("{}mo", d / (86_400 * 30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The standard fixture identity ("Test"/"test@example.com"); the
    /// hermetic env block lives once in `crate::test_support`.
    fn git(dir: &Path, args: &[&str]) -> String {
        crate::test_support::git_cli(dir, args, "Test", "test@example.com")
    }

    fn init_repo(dir: &Path) -> GitRepo {
        crate::test_support::git_repo_init(dir, "Test", "test@example.com", true);
        GitRepo::discover(dir).expect("discover the repo")
    }

    /// `n` commits, one file changed per commit, newest first.
    fn make_history(g: &GitRepo, root: &Path, n: usize) {
        for i in 0..n {
            std::fs::write(root.join("f.txt"), format!("rev {i}\n")).unwrap();
            git(root, &["add", "f.txt"]);
            git(root, &["commit", "-q", "-m", &format!("commit {i}")]);
            let _ = g; // g is not used by the CLI-driven history
        }
    }

    #[test]
    fn log_paging_matches_git_log_oneline() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        let n = 30;
        make_history(&g, root, n);

        // `git log --oneline` (newest first): "<sha7-ish> subject".
        let oneline = git(root, &["log", "--oneline"]);
        let git_subjects: Vec<&str> = oneline
            .lines()
            .map(|l| l.split_once(' ').map(|(_, s)| s).unwrap_or(l))
            .collect();
        assert_eq!(git_subjects.len(), n, "git log:\n{oneline}");

        // Page 0 (limit 10): the first 10 subjects, in order.
        let page0 = g.log(None, 0, 10).unwrap();
        assert_eq!(page0.len(), 10);
        let page0_subjects: Vec<&str> = page0.iter().map(|e| e.subject.as_str()).collect();
        assert_eq!(page0_subjects, git_subjects[..10], "page 0");

        // The short ids agree with git's leading 7 chars.
        let git_shorts: Vec<&str> = oneline
            .lines()
            .map(|l| &l[..7])
            .collect();
        for (entry, &short) in page0.iter().zip(git_shorts.iter()) {
            assert_eq!(
                entry.short_id, short,
                "short id mismatch in page 0"
            );
        }

        // Total + a later page are exact and stable.
        assert_eq!(g.log_total(None).unwrap(), n);
        let page2 = g.log(None, 20, 5).unwrap();
        assert_eq!(page2.len(), 5, "page at offset 20 should hold 5");
        let page2_subjects: Vec<&str> = page2.iter().map(|e| e.subject.as_str()).collect();
        assert_eq!(page2_subjects, git_subjects[20..25], "page at offset 20");

        // A trailing page clamps to the remaining commits.
        let tail = g.log(None, 28, 10).unwrap();
        assert_eq!(tail.len(), n - 28, "tail page must clamp");
        assert_eq!(tail[0].subject, git_subjects[28]);
    }

    #[test]
    fn log_author_and_time_are_set() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        make_history(&g, root, 3);
        let page = g.log(None, 0, 3).unwrap();
        assert!(page.iter().all(|e| e.author == "Test"));
        // Times are monotonically non-decreasing newest-first (reversed).
        assert!(page.windows(2).all(|w| w[0].time >= w[1].time));
    }

    #[test]
    fn commit_diff_matches_git_show_stat() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        // Three commits; the last touches two files.
        std::fs::write(root.join("a.txt"), "a1\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["commit", "-q", "-m", "first"]);
        std::fs::write(root.join("b.txt"), "b1\nb2\nb3\n").unwrap();
        git(root, &["add", "b.txt"]);
        git(root, &["commit", "-q", "-m", "second"]);
        std::fs::write(root.join("a.txt"), "a1\na2\na3\n").unwrap();
        std::fs::write(root.join("b.txt"), "b1\nnew\nb3\n").unwrap();
        git(root, &["add", "a.txt"]);
        git(root, &["add", "b.txt"]);
        git(root, &["commit", "-q", "-m", "third: touch two files"]);

        let full_oid = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        let short = &full_oid[..7];
        let cd = g.commit_diff(short).unwrap();

        assert_eq!(cd.short_id, short);
        assert_eq!(cd.subject, "third: touch two files");
        // Two files changed.
        assert_eq!(cd.files.len(), 2, "commit diff:\n{cd:#?}");
        let a = cd.files.iter().find(|f| f.path == "a.txt").expect("a.txt");
        let b = cd.files.iter().find(|f| f.path == "b.txt").expect("b.txt");
        // a.txt: +2 (a2, a3); b.txt: +1/-1 (new replaces b2).
        assert_eq!(a.insertions, 2, "a.txt: {a:#?}");
        assert_eq!(a.deletions, 0);
        assert_eq!(b.insertions, 1, "b.txt: {b:#?}");
        assert_eq!(b.deletions, 1);
        // Totals agree with `git show --stat`.
        let stat = git(root, &["show", "--stat", "HEAD"]);
        assert!(stat.contains("3 insertions"), "stat:\n{stat}");
        assert!(stat.contains("1 deletion"), "stat:\n{stat}");
        assert_eq!(cd.insertions, 3);
        assert_eq!(cd.deletions, 1);

        // Cross-check a body line against `git diff HEAD^ HEAD`.
        let diff = git(root, &["diff", &format!("{full_oid}^"), &full_oid]);
        assert!(diff.contains("+a2"), "a2 line missing:\n{diff}");
        assert!(diff.contains("-b2"), "-b2 missing:\n{diff}");
    }

    #[test]
    fn commit_diff_root_commit_diffs_against_empty_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        std::fs::write(root.join("root.txt"), "hello\n").unwrap();
        git(root, &["add", "root.txt"]);
        git(root, &["commit", "-q", "-m", "root"]);
        let full_oid = git(root, &["rev-parse", "HEAD"]).trim().to_string();
        let cd = g.commit_diff(&full_oid).unwrap();
        assert_eq!(cd.files.len(), 1, "root commit diff: {cd:#?}");
        assert_eq!(cd.files[0].path, "root.txt");
        assert_eq!(cd.insertions, 1, "root add is a pure insertion");
        assert_eq!(cd.deletions, 0);
    }

    #[test]
    fn relative_time_buckets() {
        let now = 1_000_000;
        assert_eq!(relative_time_from(now - 5, now), "5s");
        assert_eq!(relative_time_from(now - 3600, now), "1h");
        assert_eq!(relative_time_from(now - 86_400 * 2, now), "2d");
        assert_eq!(relative_time_from(now - 86_400 * 60, now), "2mo");
    }
}
