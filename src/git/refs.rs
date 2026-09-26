//! Branch and stash wrappers (issue 08). git2 types stay in this module;
//! the store consumes the plain `Branch` / `StashEntry` structs and drives
//! the branch/stash pickers.

use crate::git::error::GitError;
use crate::git::repo::GitRepo;

/// A local branch, with a marker for the current (HEAD) one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
}

/// One stash entry (`index` 0 = newest, matching `git stash list`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub index: usize,
    pub subject: String,
}

impl GitRepo {
    /// All local branches, sorted by name, with the current (HEAD) branch
    /// marked.
    pub fn branches(&self) -> Result<Vec<Branch>, GitError> {
        let head = self
            .inner
            .head()
            .ok()
            .and_then(|r| r.shorthand().ok().map(|s| s.to_string()));
        let mut out = Vec::new();
        let branches = self.inner.branches(Some(git2::BranchType::Local))?;
        for b in branches {
            let (branch, _type) = b?;
            let name = branch
                .name()
                .map(|n| n.unwrap_or_default().to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let current = Some(name.as_str()) == head.as_deref();
            out.push(Branch { name, current });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Check out the local branch `name`: update the workdir + index to its
    /// tree, then move HEAD. Magit's default guard is honoured: if the
    /// working tree or index has any staged/unstaged change, the switch is
    /// refused with [`GitError::DirtyTree`] (untracked files do not block a
    /// switch, mirroring `git checkout`'s collision rule).
    pub fn checkout_branch(&self, name: &str) -> Result<(), GitError> {
        let status = self.status()?;
        if status.staged_count() + status.unstaged_count() > 0 {
            return Err(GitError::DirtyTree);
        }
        let branch = self
            .inner
            .find_branch(name, git2::BranchType::Local)?;
        let target = branch.get().peel_to_commit()?;
        let obj = target.as_object();
        // A non-force checkout: libgit2 updates the index + workdir to the
        // target tree, refusing to clobber uncommitted changes.
        let mut cb = git2::build::CheckoutBuilder::new();
        self.inner.checkout_tree(obj, Some(&mut cb))?;
        self.inner.set_head(&format!("refs/heads/{name}"))?;
        Ok(())
    }

    /// Create a new local branch at the current HEAD (does not check it out).
    pub fn create_branch_from_head(&self, name: &str) -> Result<(), GitError> {
        let head_commit = self.inner.head()?.peel_to_commit()?;
        self.inner.branch(name, &head_commit, false)?;
        Ok(())
    }

    /// The stash list (`index` 0 = newest), subject = git's stash message.
    pub fn stash_list(&mut self) -> Result<Vec<StashEntry>, GitError> {
        let mut out = Vec::new();
        self.inner
            .stash_foreach(|index, message, _oid| {
                out.push(StashEntry {
                    index,
                    subject: message.to_string(),
                });
                true
            })?;
        Ok(out)
    }

    /// Pop (apply + drop) the stash at `index`.
    pub fn stash_pop(&mut self, index: usize) -> Result<(), GitError> {
        self.inner.stash_pop(index, None)?;
        Ok(())
    }

    /// Drop the stash at `index` (without applying it).
    pub fn stash_drop(&mut self, index: usize) -> Result<(), GitError> {
        self.inner.stash_drop(index)?;
        Ok(())
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
        std::fs::write(dir.join("f.txt"), "v1\n").unwrap();
        git(dir, &["add", "f.txt"]);
        git(dir, &["commit", "-q", "-m", "init"]);
        GitRepo::discover(dir).expect("discover the repo")
    }

    #[test]
    fn branches_lists_all_with_current_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        // Two more branches off main.
        git(root, &["branch", "feature"]);
        git(root, &["branch", "release"]);

        let branches = g.branches().unwrap();
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        // Sorted by name; main is current.
        assert_eq!(names, vec!["feature", "main", "release"], "{names:?}");
        let marks: Vec<bool> = branches.iter().map(|b| b.current).collect();
        assert_eq!(marks, vec![false, true, false], "main must be current");

        // Cross-check against `git branch --format=%(refname:short)`.
        let git_list = git(root, &["branch", "--format=%(refname:short)"]);
        let git_names: Vec<&str> = git_list.lines().collect();
        let mut sorted = git_names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "wrapper vs git branch list");
    }

    #[test]
    fn clean_checkout_updates_head_and_refuses_when_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        git(root, &["branch", "other"]);

        // Dirty tree: a workdir change blocks the switch (magit default).
        std::fs::write(root.join("f.txt"), "dirty\n").unwrap();
        let err = g.checkout_branch("other").unwrap_err();
        assert!(matches!(err, GitError::DirtyTree), "{err:?}");
        // HEAD is still on main and the index/workdir are unchanged.
        assert_eq!(git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "dirty\n");

        // Clean tree: the switch succeeds and both sides move to `other`.
        // (restore the committed content so the tree is clean)
        std::fs::write(root.join("f.txt"), "v1\n").unwrap();
        g.checkout_branch("other").unwrap();
        assert_eq!(
            git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "other"
        );
        let branches = g.branches().unwrap();
        let other = branches.iter().find(|b| b.name == "other").unwrap();
        assert!(other.current, "other must be current after checkout");
    }

    #[test]
    fn create_branch_from_head_matches_git_branch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let g = init_repo(root);
        g.create_branch_from_head("new-branch").unwrap();
        let branches = g.branches().unwrap();
        assert!(
            branches.iter().any(|b| b.name == "new-branch"),
            "new-branch missing: {branches:#?}"
        );
        // It points at HEAD (same commit as main).
        let a = git(root, &["rev-parse", "main"]);
        let b = git(root, &["rev-parse", "new-branch"]);
        assert_eq!(a, b, "new branch must equal HEAD at creation");
    }

    fn make_stash(dir: &Path) {
        // A stashed workdir change.
        std::fs::write(dir.join("f.txt"), "stashed\n").unwrap();
        git(dir, &["stash", "push", "-m", "my stash"]);
    }

    #[test]
    fn stash_list_pop_and_drop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut g = init_repo(root);
        make_stash(root);

        let list = g.stash_list().unwrap();
        assert_eq!(list.len(), 1, "stash list: {list:#?}");
        assert_eq!(list[0].index, 0);
        assert!(list[0].subject.contains("my stash"), "{:?}", list[0].subject);

        // `git stash list` agrees (subject text is git's own message).
        let git_list = git(root, &["stash", "list"]);
        assert!(git_list.contains("my stash"), "git stash list:\n{git_list}");

        // Pop: the working change is restored and the entry is removed.
        g.stash_pop(0).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "stashed\n",
            "pop must restore the stashed workdir change"
        );
        assert!(git(root, &["diff"]).contains("+stashed"), "restored change must be unstaged");
        assert!(g.stash_list().unwrap().is_empty(), "pop must drop the entry");
    }

    #[test]
    fn stash_drop_removes_entry_without_applying() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut g = init_repo(root);
        make_stash(root);
        assert_eq!(g.stash_list().unwrap().len(), 1);
        g.stash_drop(0).unwrap();
        assert!(g.stash_list().unwrap().is_empty(), "drop must remove the entry");
        // The workdir change was NOT restored.
        assert!(git(root, &["diff"]).trim().is_empty(), "drop must not apply the change");
    }

    #[test]
    fn stash_empty_is_a_clean_noop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut g = init_repo(root);
        assert!(g.stash_list().unwrap().is_empty(), "fresh repo has no stashes");
        // Popping/dropping an empty stash index is out of range; the list is
        // simply empty (the picker disables the action on an empty list).
        assert_eq!(git(root, &["stash", "list"]).trim(), "");
    }
}
