//! Cross-check: every PTY drive is registered in the gate, or explicitly
//! allowlisted as manual-only.
//!
//! WHY. A `tools/drive_*.py` that is in neither `tools/gate.sh`'s
//! SHARED_SUITES/HEAVY_SUITES nor `tools/pool.py`'s BATTERY is OUTSIDE the
//! battery: its failures are invisible to the runs we actually do, so it
//! silently rots as the app changes under it. This is the same disease as the
//! registry dispatch drift fixed in 0829ddd (a hand-maintained list out of
//! step with reality) — and the durable fix is the same: make the drift
//! self-diagnosing rather than relying on a periodic manual audit.
//!
//! The test reads the two registration sites STRAIGHT (gate.sh's arrays,
//! pool.py's list) instead of a copy of them: a pin that read a *copy* of the
//! fact would be the false-assurance this repo has already been bitten by.
//! The manual-only set is an explicit allowlist with a per-drive reason, so
//! "not registered" stops being ambiguous. Adding a `tools/drive_*.py` and
//! forgetting to register it is now a red test, not a silent hole.
//!
//! Load-bearing by mutation: delete any drive from SHARED_SUITES + BATTERY and
//! drop it from MANUAL_ONLY and `every_drive_is_registered_or_manual_only`
//! reddens naming that drive; a parser regression that moves a marker fails
//! the marker-presence asserts LOUD (not as a wall of red rows); a
//! registration pointing at a deleted file reddens
//! `registered_drive_suites_exist_on_disk`.

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::Path;

    const GATE: &str = "gate.sh";
    const POOL: &str = "pool.py";

    /// Drives that are DELIBERATELY outside the battery, with the per-drive
    /// reason so "not registered" stops being ambiguous. A drive named here is
    /// exempt from `every_drive_is_registered_or_manual_only`.
    ///
    /// The set splits into two kinds, both manual-only:
    ///   * the emacs side of the parity capture (runs a live `emacs -Q -nw`
    ///     30.2 as the oracle — not hermetic); and
    ///   * the redline side of that same capture (hermetic to run, but carries
    ///     NO self-contained assertions — its value is only in a manual
    ///     diff against the emacs capture, so it is not a pass/fail gate).
    const MANUAL_ONLY: &[(&str, &str)] = &[
        ("drive_emacs.py",
         "runs a live `emacs -Q -nw` 30.2 as the parity oracle (not hermetic); a reference capture — the pass/fail assertions live in the redline drive it is compared against"),
        ("drive_emacs_battery2.py",
         "runs a live `emacs -Q -nw` 30.2 (imports drive_emacs::EmacsSession); parity capture, no self-contained assertions"),
        ("drive_emacs_battery3.py",
         "runs a live `emacs -Q -nw` 30.2 (imports drive_emacs::EmacsSession); parity capture, no self-contained assertions"),
        ("drive_redline_parity.py",
         "redline half of the emacs<->redline parity capture (mirror of drive_emacs.py); compared manually against the emacs capture, no self-contained assertions. STALE: imports the missing `uxdrive` module, so it crashes on import"),
        ("drive_redline_battery2.py",
         "redline half of the battery2 parity capture (mirror of drive_emacs_battery2.py); manual comparison, no self-contained assertions. STALE: imports the missing `uxdrive` module, so it crashes on import"),
        ("drive_redline_battery3.py",
         "redline half of the battery3 parity capture (mirror of drive_emacs_battery3.py); manual comparison, no self-contained assertions"),
    ];

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    fn tools_dir() -> std::path::PathBuf {
        repo_root().join("tools")
    }

    fn read_tools(name: &str) -> String {
        std::fs::read_to_string(tools_dir().join(name))
            .unwrap_or_else(|e| panic!("cannot read tools/{name} ({e})"))
    }

    /// Every `tools/drive_*.py` file that exists on disk (the ground truth the
    /// registration lists must account for).
    fn drive_files() -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(tools_dir())
            .expect("tools/ directory unreadable")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("drive_") && n.ends_with(".py"))
            .collect();
        out.sort();
        out
    }

    /// Collect the `*.py` tokens out of one line of a bash array, dropping the
    /// inline `#` comment (so a `)` inside a comment can never be mistaken for
    /// the array's closing paren).
    fn collect_suites(line: &str, out: &mut Vec<String>) {
        let line = match line.find('#') {
            Some(i) => &line[..i],
            None => line,
        };
        for tok in line.split_whitespace() {
            let tok = tok.trim();
            // A bare `*.py` filename token (alphanumerics, `_`, `.`). The `.`
            // is required — the suite names themselves carry one.
            if tok.ends_with(".py")
                && tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            {
                out.push(tok.to_string());
            }
        }
    }

    /// Extract a bash `NAME=( ... )` array's suite entries from gate.sh.
    /// Handles both the single-line form (`HEAVY_SUITES=(sweep_flows.py)`) and
    /// the multi-line form (SHARED_SUITES, whose closing `)` is on its own
    /// line; only a line that is EXACTLY `)` closes it, so a `)` in a comment
    /// cannot truncate the list). Fails loudly if the marker moved.
    fn bash_array_suites(gate: &str, name: &str) -> Vec<String> {
        let open = format!("{name}=(");
        let idx = gate.find(open.as_str()).unwrap_or_else(|| {
            panic!(
                "gate.sh: `{open}` not found — the registration marker moved; \
                 the cross-check can no longer read the gate's suite list"
            )
        });
        let rest = &gate[idx + open.len()..];
        let mut out = Vec::new();
        let mut lines = rest.split('\n');
        let first = lines.next().unwrap_or("");
        // Single-line array: the closing paren is on the first (continuation) line.
        if let Some(pos) = first.find(')') {
            collect_suites(&first[..pos], &mut out);
            return out;
        }
        // Multi-line: the first line is the content after `(`, then the array
        // closes at the first line that is exactly `)`.
        collect_suites(first, &mut out);
        for line in lines {
            let trimmed = line.trim();
            if trimmed == ")" {
                break;
            }
            collect_suites(trimmed, &mut out);
        }
        out
    }

    /// Extract the `NAME = [ ... ]` list's entries from pool.py (each is a
    /// double-quoted `*.py` string). Fails loudly if the marker moved.
    fn py_list_suites(pool: &str, name: &str) -> Vec<String> {
        let needle = format!("{name} = [");
        let start = pool.find(needle.as_str()).unwrap_or_else(|| {
            panic!(
                "pool.py: `{needle}` not found — the BATTERY marker moved; \
                 the cross-check can no longer read the pooled battery list"
            )
        });
        let mut depth = 0usize;
        let mut end = start;
        for (i, ch) in pool[start..].char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        // The odd segments after splitting on `"` are the quoted suite names.
        pool[start..end]
            .split('"')
            .filter(|seg| seg.ends_with(".py"))
            .map(|seg| seg.to_string())
            .collect()
    }

    /// PRIMARY. Every drive on disk must be registered in at least one list
    /// (gate.sh's SHARED/HEAVY, or pool.py's BATTERY) OR explicitly allowlisted
    /// as manual-only. A drive in neither is outside the battery and its
    /// failures are invisible — that is the hole this exists to catch.
    #[test]
    fn every_drive_is_registered_or_manual_only() {
        let gate = read_tools(GATE);
        let pool = read_tools(POOL);
        let shared = bash_array_suites(&gate, "SHARED_SUITES");
        let heavy = bash_array_suites(&gate, "HEAVY_SUITES");
        let battery = py_list_suites(&pool, "BATTERY");

        // Marker presence, pinned LOUD: a parser regression that yields an
        // empty list would otherwise surface as every drive "unregistered".
        assert!(
            !shared.is_empty(),
            "SHARED_SUITES parsed empty — the gate.sh marker moved or the parser regressed"
        );
        assert!(
            !battery.is_empty(),
            "BATTERY parsed empty — the pool.py marker moved or the parser regressed"
        );

        let registered: HashSet<&str> = shared
            .iter()
            .chain(heavy.iter())
            .chain(battery.iter())
            .map(|s| s.as_str())
            .collect();
        let manual: HashSet<&str> = MANUAL_ONLY.iter().map(|(n, _)| *n).collect();

        for d in drive_files() {
            assert!(
                registered.contains(d.as_str()) || manual.contains(d.as_str()),
                "drive `{d}` is in NEITHER registration list (gate.sh SHARED_SUITES / \
                 HEAVY_SUITES, pool.py BATTERY) nor the manual-only allowlist in \
                 src/gate_registry.rs. It is outside the battery: its failures are \
                 invisible to the runs we actually do. Register it in gate.sh and/or \
                 pool.py, or add it to MANUAL_ONLY with a one-line reason."
            );
        }
    }

    /// Every entry in the manual-only allowlist must be a real drive file on
    /// disk (catches a stale or misspelled allowlist entry, which would silently
    /// exempt a name that no longer exists — or mask a new drive by typo).
    #[test]
    fn manual_only_entries_are_real_drives() {
        let drives = drive_files();
        for (name, reason) in MANUAL_ONLY {
            assert!(
                drives.iter().any(|d| d == name),
                "MANUAL_ONLY lists `{name}` ({reason}) but there is no tools/{name} — \
                 a stale or misspelled allowlist entry"
            );
        }
    }

    /// Every `drive_*.py` named in the registration lists must exist on disk.
    /// Catches a registration pointing at a deleted drive, which would make the
    /// gate fail cryptically (`timeout … python3 tools/…: can't open file`).
    #[test]
    fn registered_drive_suites_exist_on_disk() {
        let gate = read_tools(GATE);
        let pool = read_tools(POOL);
        let shared = bash_array_suites(&gate, "SHARED_SUITES");
        let heavy = bash_array_suites(&gate, "HEAVY_SUITES");
        let battery = py_list_suites(&pool, "BATTERY");

        let mut seen = HashSet::new();
        for name in shared.iter().chain(heavy.iter()).chain(battery.iter()) {
            if !name.starts_with("drive_") {
                continue;
            }
            if !seen.insert(name.as_str()) {
                continue;
            }
            assert!(
                tools_dir().join(name).exists(),
                "registration lists `{name}` but tools/{name} does not exist — a \
                 registration pointing at a deleted drive (the gate would fail cryptically)"
            );
        }
    }
}
