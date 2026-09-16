//! Project layer: root detection, project identity, and the persisted
//! project stores (known-project registry + per-project recents) under
//! the redline cache directory.
//!
//! Root detection (plan 02): the innermost directory holding a `.git`
//! (a directory in normal repos, a *file* in worktrees/submodules) wins;
//! otherwise the nearest directory with any marker file. Documented
//! deviation from projectile: redline always prefers the git root over
//! a nearer-or-outer `.projectile`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Marker files identifying a project root when no `.git` is found
/// on the way up.
const MARKERS: &[&str] =
    &["Cargo.toml", "package.json", "pyproject.toml", "go.mod", ".projectile"];

/// One project: its canonical root path plus a display name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
}

impl Project {
    /// A project rooted at `root`; the name is the root's file name
    /// (or the path itself when the root is a filesystem root).
    pub fn new(root: PathBuf) -> Self {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Self { root, name }
    }
}

/// Detect the project root for a starting directory: the innermost
/// `.git` on the way up; else the nearest marker file. `None` when no
/// project encloses `start`.
pub fn detect_root(start: &Path) -> Option<PathBuf> {
    // Pass 1: innermost git root (`.git` dir or file).
    for dir in start.ancestors() {
        if dir.join(".git").exists() {
            return canonicalize(dir);
        }
    }
    // Pass 2: nearest marker file.
    for dir in start.ancestors() {
        if MARKERS.iter().any(|m| dir.join(m).exists()) {
            return canonicalize(dir);
        }
    }
    None
}

fn canonicalize(dir: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(dir).ok()
}

/// Known-project registry, most-recently-used first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Registry {
    pub projects: Vec<Project>,
}

#[derive(Serialize, Deserialize)]
struct RegistryFile {
    projects: Vec<Project>,
}

impl Registry {
    /// Load from `path`; a missing or corrupt file yields an empty
    /// registry (the next mutation re-saves a clean one).
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(|f: RegistryFile| Self {
                    projects: f.projects,
                })
                .unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_json(path, &RegistryFile {
            projects: self.projects.clone(),
        })
    }

    /// Add `root` (or move it to the front) and return the entry.
    pub fn upsert(&mut self, root: &Path) -> Project {
        let project = Project::new(root.to_path_buf());
        self.projects.retain(|p| p.root != project.root);
        self.projects.insert(0, project.clone());
        project
    }

    pub fn list(&self) -> &[Project] {
        &self.projects
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }
}

/// Per-project recently-opened files (relative paths, MRU first).
const RECENTS_CAP: usize = 100;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recents {
    per_project: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize, Deserialize, Default)]
struct RecentsFile(BTreeMap<String, Vec<String>>);

impl Recents {
    /// Load from `path`; a missing or corrupt file yields empty recents.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(|f: RecentsFile| Self {
                    per_project: f.0,
                })
                .unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_json(path, &RecentsFile(self.per_project.clone()))
    }

    /// Record `rel` as recently opened in the project rooted at `root`;
    /// dedupes (moves to the front) and caps the list.
    pub fn add(&mut self, root: &str, rel: &str) {
        let list = self.per_project.entry(root.to_string()).or_default();
        list.retain(|p| p != rel);
        list.insert(0, rel.to_string());
        list.truncate(RECENTS_CAP);
    }

    /// Recents for the project rooted at `root` (MRU first); `[]` if
    /// the project has no recents.
    pub fn list(&self, root: &str) -> &[String] {
        self.per_project
            .get(root)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)
}

/// The persisted project state — registry + recents — under one base
/// directory. Production uses `cache_dir()/redline`; tests inject a
/// tempdir so the real cache path is never hardcoded.
pub struct ProjectStore {
    base: PathBuf,
    pub registry: Registry,
    pub recents: Recents,
}

impl ProjectStore {
    /// Open (tolerantly loading) the stores under `base`.
    pub fn open(base: PathBuf) -> Self {
        Self {
            registry: Registry::load(&base.join("projects.json")),
            recents: Recents::load(&base.join("recents.json")),
            base,
        }
    }

    /// `cache_dir()/redline` (via `dirs`); falls back to `./redline`
    /// when the cache dir is unavailable.
    pub fn default_base() -> PathBuf {
        dirs::cache_dir()
            .map(|d| d.join("redline"))
            .unwrap_or_else(|| PathBuf::from("redline"))
    }

    pub fn registry_path(&self) -> PathBuf {
        self.base.join("projects.json")
    }

    pub fn recents_path(&self) -> PathBuf {
        self.base.join("recents.json")
    }

    pub fn save_registry(&self) -> std::io::Result<()> {
        self.registry.save(&self.registry_path())
    }

    pub fn save_recents(&self) -> std::io::Result<()> {
        self.recents.save(&self.recents_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn file(path: impl AsRef<std::path::Path>, content: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn git_root_wins_over_markers_above() {
        let dir = tempfile::tempdir().unwrap();
        // Outer project (marker), inner git repo, deep start dir.
        file(dir.path().join("Cargo.toml"), "[package]\n");
        fs::create_dir_all(dir.path().join("inner/.git")).unwrap();
        let deep = dir.path().join("inner/src");
        fs::create_dir_all(&deep).unwrap();
        file(deep.join("main.rs"), "fn main() {}\n");

        let root = detect_root(&deep).expect("a project must be found");
        assert_eq!(root, fs::canonicalize(dir.path().join("inner")).unwrap());
    }

    #[test]
    fn git_file_counts_as_git_root() {
        // Worktrees/submodules store `.git` as a file.
        let dir = tempfile::tempdir().unwrap();
        file(dir.path().join(".git"), "gitdir: /elsewhere\n");
        let sub = dir.path().join("a/b");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(detect_root(&sub), Some(fs::canonicalize(dir.path()).unwrap()));
    }

    #[test]
    fn marker_fallback_finds_nearest_manifest() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path().join("pyproject.toml"), "[project]\n");
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        assert_eq!(detect_root(&src), Some(fs::canonicalize(dir.path()).unwrap()));
    }

    #[test]
    fn each_marker_is_recognized() {
        for marker in ["Cargo.toml", "package.json", "pyproject.toml", "go.mod", ".projectile"]
        {
            let dir = tempfile::tempdir().unwrap();
            file(dir.path().join(marker), "x\n");
            let sub = dir.path().join("sub");
            fs::create_dir_all(&sub).unwrap();
            assert_eq!(
                detect_root(&sub),
                Some(fs::canonicalize(dir.path()).unwrap()),
                "marker `{marker}` not detected"
            );
        }
    }

    #[test]
    fn nearest_marker_wins() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path().join("Cargo.toml"), "[package]\n");
        let nested = dir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        file(nested.join("package.json"), "{}\n");
        assert_eq!(detect_root(&nested), Some(fs::canonicalize(&nested).unwrap()));
    }

    #[test]
    fn no_project_when_no_marker_encloses() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c");
        fs::create_dir_all(&deep).unwrap();
        assert_eq!(detect_root(&deep), None);
    }

    #[test]
    fn project_name_is_root_file_name() {
        let p = Project::new(PathBuf::from("/home/u/red"));
        assert_eq!(p.name, "red");
        let p = Project::new(PathBuf::from("/"));
        assert_eq!(p.name, "/");
    }

    #[test]
    fn registry_roundtrip_and_mru_order() {
        let base = tempfile::tempdir().unwrap();
        let path = base.path().join("projects.json");
        let p1 = Project::new(PathBuf::from("/a/proj1"));
        let p2 = Project::new(PathBuf::from("/a/proj2"));
        assert_eq!(p1.name, "proj1");
        assert_eq!(p2.name, "proj2");

        let mut reg = Registry::default();
        reg.upsert(Path::new("/a/proj1"));
        reg.upsert(Path::new("/a/proj2"));
        reg.upsert(Path::new("/a/proj1")); // re-upsert: dedupe + move to front
        reg.save(&path).unwrap();

        let loaded = Registry::load(&path);
        assert_eq!(loaded, reg);
        let names: Vec<_> = loaded.list().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["proj1", "proj2"]);
    }

    #[test]
    fn registry_missing_or_corrupt_file_is_empty() {
        let base = tempfile::tempdir().unwrap();
        assert!(Registry::load(&base.path().join("nope.json")).list().is_empty());

        let path = base.path().join("projects.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(Registry::load(&path).list().is_empty());
    }

    #[test]
    fn recents_mru_cap_and_roundtrip() {
        let mut r = Recents::default();
        r.add("/root", "b.rs");
        r.add("/root", "a.rs");
        r.add("/root", "b.rs"); // dedupe: moves to front
        assert_eq!(r.list("/root"), vec!["b.rs", "a.rs"]);
        assert!(r.list("/other").is_empty());

        for i in 0..150 {
            r.add("/root", &format!("f{i:03}.rs"));
        }
        assert_eq!(r.list("/root").len(), 100, "cap must hold");
        assert_eq!(r.list("/root")[0], "f149.rs", "most recent first");

        let base = tempfile::tempdir().unwrap();
        let path = base.path().join("recents.json");
        r.save(&path).unwrap();
        assert_eq!(Recents::load(&path), r);
        assert!(Recents::load(&base.path().join("missing.json")).list("/root").is_empty());
    }

    #[test]
    fn project_store_paths_and_default_base() {
        let base = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::open(base.path().join("redline-cache"));
        assert_eq!(store.registry_path().file_name().unwrap(), "projects.json");
        assert_eq!(store.recents_path().file_name().unwrap(), "recents.json");
        store.registry.upsert(Path::new("/x/y"));
        store.save_registry().unwrap();
        assert!(store.registry_path().is_file());
        assert_eq!(Project::new(store.registry.list()[0].root.clone()).name, "y");
        // Production default lives under the cache dir (or ./redline).
        let d = ProjectStore::default_base();
        assert!(d.file_name().map(|n| n == "redline").unwrap_or(false));
    }
}
