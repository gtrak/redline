//! Plan 002 issue 03 (Part C): re-measure the README perf table.
//!
//! Method (the committed method of record — git history contains no prior
//! harness for these numbers): a store-level microbench driving the
//! production code paths headlessly (no PTY, no render loop), on a
//! synthetic 500-file Rust repo (each ~50 lines) in a tempdir with a fresh
//! `.git` dir so project detection takes the git path. Cold cache
//! (fresh `ProjectStore` base dir per run). Median of 5 runs.
//!
//! Metrics, matching the README table's rows:
//!   * Cold start (store init): `AppStore::at` (project detection + store
//!     construction) + the initial project file walk (`FileList::build`,
//!     the same walk the store runs via `ensure_files` before anything can
//!     render candidates).
//!   * Index (500 files): `start_indexing` until the final `IndexEvent`
//!     (`indexing == false`). The store's own initial walk runs inside this
//!     phase (a few ms at 500 files; noted, not subtracted).
//!   * Search first-hit: `start_project_search` until the first
//!     `SearchEvent::Hit` on the search bus.
//!
//! Run:
//!   cargo test -- perf_remeasure -- --include-ignored --nocapture      (debug)
//!   cargo test --release -- perf_remeasure -- --include-ignored --nocapture (release)

#[cfg(test)]
mod tests {
    use crate::app::store::AppStore;
    use crate::model::files::FileList;
    use crate::search::rg::SearchEvent;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const FILES: usize = 500;
    const LINES_PER_FILE: usize = 50;
    const RUNS: usize = 5;

    /// Build the synthetic 500-file Rust repo (each ~50 lines). Exactly one
    /// file carries the search query, so the first-hit time is a real
    /// walk-and-match measurement, not an instant prefix match.
    fn build_repo(dir: &Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // Project detection (git path) + a dir the file walk must prune.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        for i in 0..FILES {
            let mut body = String::new();
            if i == 0 {
                body.push_str("fn target_marker() {\n    let x = 1u32;\n    x + 1\n}\n");
            }
            for l in 0..LINES_PER_FILE {
                body.push_str(&format!("fn filler_{i}_{l}() {{ let v = {l}u32; v * 2 }}\n"));
            }
            std::fs::write(dir.join("src").join(format!("f_{i:03}.rs")), body).unwrap();
        }
    }

    fn median(xs: &[f64]) -> f64 {
        let mut v = xs.to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }

    #[ignore]
    #[tokio::test]
    async fn perf_remeasure_500_file_repo() {
        let repo = tempfile::tempdir().unwrap();
        build_repo(repo.path());

        let mut init_ms = Vec::new();
        let mut index_ms = Vec::new();
        let mut first_hit_ms = Vec::new();

        for _ in 0..RUNS {
            let cache = tempfile::tempdir().unwrap(); // cold cache per run
            // Phase 1: cold start (store init + initial file walk).
            let t0 = Instant::now();
            let mut store = AppStore::at(repo.path(), cache.path().to_path_buf());
            assert!(
                store.project.is_some(),
                "project detection must find the git root"
            );
            let walk = FileList::build(repo.path()).unwrap();
            assert_eq!(walk.len(), FILES, "file walk must see all {FILES} files");
            init_ms.push(t0.elapsed().as_secs_f64() * 1e3);

            // Phase 2: index build until the final (indexing == false) event.
            let mut rx = store.index_bus.subscribe();
            let t1 = Instant::now();
            store.start_indexing();
            tokio::time::timeout(Duration::from_secs(120), async {
                loop {
                    rx.changed().await.expect("index event");
                    if !rx.borrow().indexing {
                        break;
                    }
                }
            })
            .await
            .expect("index build timed out");
            index_ms.push(t1.elapsed().as_secs_f64() * 1e3);

            // Phase 3: search first-hit.
            let mut srx = store.search_rx().expect("search receiver");
            let t2 = Instant::now();
            store.start_project_search("target_marker".to_string());
            tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    match srx.recv().await {
                        Some(SearchEvent::Hit { .. }) => break,
                        Some(SearchEvent::Error { message, .. }) => {
                            panic!("search error: {message}")
                        }
                        Some(_) => {}
                        None => panic!("search bus closed"),
                    }
                }
            })
            .await
            .expect("search first-hit timed out");
            first_hit_ms.push(t2.elapsed().as_secs_f64() * 1e3);
        }

        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        let (i, ix, fh) = (
            median(&init_ms),
            median(&index_ms),
            median(&first_hit_ms),
        );
        println!("PERF RE-MEASURE (500-file Rust repo, {profile}, cold cache, median of {RUNS}):");
        println!("  cold start (store init + file walk): {i:7.1} ms");
        println!("  index (500 files):                   {ix:7.1} ms   ({:5.0} files/s)", FILES as f64 * 1e3 / ix);
        println!("  search first-hit:                    {fh:7.2} ms");
        println!("  (per-run) init={init_ms:?} index={index_ms:?} first_hit={first_hit_ms:?}");
    }
}
