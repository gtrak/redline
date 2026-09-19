# Go golden corpus (plan 011, issue 08)

A small, genuine Go module (`project/`) plus a module-cache-shaped
directory layout (`modcache/`) exercising the GO provider
(`crates/redline-resolve/src/providers/go_provider.rs`) end to end,
with every probe's expected outcome checked in as a `*.golden` file.
`tests/golden_go.rs` walks the goldens in sorted order; a behavior
change in the provider or the resolution seam shows up as a deliberate
golden diff — never a silent regression. The corpus OBSERVES the
provider; a real bug it exposes is a finding, not a fix, in this issue.

## LAYOUT-BASED — no go toolchain (read first)

**Every outcome in this corpus is LAYOUT-BASED, not live-toolchain.**
The go toolchain is ABSENT in the gate sandbox. The provider's module
cache is INJECTED via `with_mod_cache(corpus/modcache)` on an
`.offline()` provider, so `go env GOMODCACHE` and `go mod download` are
never invoked. `modcache/` is a checked-in, module-cache-shaped
directory tree (the exact shape a live `go mod download` would
produce — module paths case-encoded as `!lowercase` per
go.dev/ref/mod, e.g. `github.com/MyOrg/mylib` →
`github.com/!my!org/!my!lib`), not the product of a live `go` run.
No probe in this corpus is a live-toolchain result; a future live lane
must re-verify the landings against a real `GOMODCACHE`. No PTY
anywhere.

## Corpus layout

```
project/                    # probe root (`root=project` in 16 of 17 goldens)
  go.mod                    # module github.com/redlinecorp/gearserv
  go.sum
  app.go                    # root package `gearserv` (NewApp :24) + errors.New/Wrap use sites
  cmd/gearserv/main.go      # the real USE SITES: grouped import (mylib),
                            #   aliased fork (lib), workspace-root (gearserv.NewApp),
                            #   intra-module (util), stdlib (fmt.Println)
  cmd/gearserv/tokens.go    # jwt.Parse use site (/vN-suffixed module)
  internal/util/util.go     # intra-module package (ReadManifest, Validate)
  internal/cache/cache.go   # sessionstore.Dial use site (required, cache dir absent)
  internal/audit/audit.go   # ALIASED import `pe "github.com/pkg/errors"` (probe 08)
  pkg/report/report.go      # DOT import of auxversion; bare BuildVersion +
                            #   ambiguous bare Version (probes 04/05/14)
  pkg/report/stamp.go       # package-level report.Version (the ambiguity twin)
  pkg/note/note.go          # dot-imported errors.New used bare, no hint (probe 13)
  experimental/metrics.go   # deliberately orphaned: gin import NOT in go.mod (probe 12)
  forks/lib/lib.go          # local fork behind `replace github.com/old/lib => ./forks/lib`
modcache/                   # INJECTED module-cache layout (the LAYOUT)
  github.com/pkg/errors@v0.9.1/          (errors.go, wrap.go, stack.go, go.mod)
  github.com/!my!org/mylib@v1.2.3/     (codec.go, go.mod — case-encoded dir)
  github.com/redlinecorp/auxversion@v0.4.0/ (version.go, go.mod)
  # NOTE: github.com/redlinecorp/sessionstore@v0.2.0 is intentionally ABSENT
  # (probe 15 pins the offline refusal of the fetch leg).
not_a_module/               # probe root with no go.mod (probe 17)
  scratch.go
```

## Golden format

Key/value lines (`#` = comment), one file per probe — the file is BOTH
the probe spec and the expected outcome (the python-lane shape):

| key | meaning |
|---|---|
| `symbol` | the dot-qualified or bare symbol as the app passes it |
| `from_file` | probe-root-relative file the use site lives in (context; the provider does not read it) |
| `scope` | the app's tree-sitter import-context hint (package path, item included); `[]` = no hint |
| `root` | probe workspace root, corpus-root-relative (default `project`) |
| `kind` | `resolved` or `bail` |
| `external` / `file` / `source_root` / `line` | the expected `ResolvedSource` (resolved only), paths corpus-root-relative; `line` is 1-based |
| `bail` | the exact expected bail message, byte-for-byte (bail only); `{root}` / `{cache}` placeholders for the dynamic absolute paths (probe root / injected modcache) |

Every golden header carries a `# LAYOUT-BASED: …` marker — the
manifest-consistency test asserts it, so a copy that drops the marker
goes red.

## Probe table

| probe | symbol / scope | shape | pins |
|---|---|---|---|
| 01 | `errors.New` / `[]` | resolved | plain import + qualified use, external func landing |
| 02 | `mylib.NewCodec` / `[]` | resolved | grouped import + case-encoded module path |
| 03 | `mylib.Codec` / `[]` | resolved | `type` def-shape landing |
| 04 | `BuildVersion` / `["auxversion","BuildVersion"]` | resolved | dot import, bare use WITH hint (007-03) |
| 05 | `Version` / `["auxversion","Version"]` | resolved | ambiguous dot-import, disambiguated-hint twin |
| 06 | `gearserv.NewApp` / `[]` | resolved | workspace-local root package (external=false) |
| 07 | `lib.MintToken` / `[]` | resolved | local-path `replace` (external=false, fork) |
| 08 | `pe.Wrap` / `["errors","Wrap"]` | bail | **FINDING**: aliased import + REAL hint still bails — the go provider never applies the 011-02 alias rewrite (`scope_qualified_alias`); js_provider does. If the provider gains the leg, this flips to a resolved landing in `wrap.go` |
| 09 | `fmt.Println` / `[]` | bail | stdlib corner: no GOROOT leg, std package is not a go.mod require |
| 10 | `util.Validate` / `[]` | bail | last-segment rule: intra-module imports bail as unrequired |
| 11 | `jwt.Parse` / `[]` | bail | `/vN` major-version suffix breaks the last-segment mapping |
| 12 | `gin.New` / `[]` | bail | module-not-in-go.mod (canonical) |
| 13 | `New` / `[]` | bail | bare symbol, no hint |
| 14 | `Version` / `[]` | bail | ambiguous dot import → handoff emits no hint |
| 15 | `sessionstore.Dial` / `[]` | bail | missing cache dir + offline refusal of the fetch leg |
| 16 | `errors.NotDefined` / `[]` | bail | module present, item absent |
| 17 | `fmt.Println` / `[]`, root `not_a_module` | bail | no go.mod under the probe root |

## Findings the corpus exposes (report, do not fix — this issue)

1. **Probe 08 (real gap vs. the seam contract)**: `go_provider.rs` calls
   only `scope_qualified` (bare-symbol hint); it never calls
   `scope_qualified_alias`, so a PATH-SHAPED symbol whose first segment
   is a local import alias (`pe.Wrap` from `import pe "github.com/
   pkg/errors"`, hint `["errors","Wrap"]`) is parsed as
   package `pe` and bails. `js_provider.rs` composes both. Probe 08
   carries the REAL hint deliberately (the python-lane P2 lesson): an
   empty-scope probe would bail byte-identically and pin nothing after
   a fix.
2. **Observation (not golden-pinned)**: the go.sum version fallback in
   `go_provider.rs` is unreachable through `parse_go_mod` — a require
   line without a version is dropped by `parse_require_line`, so the
   `.filter(|v| !v.is_empty()).or_else(go.sum)` leg never fires.
3. **Limitations pinned, not bugs** (documented in the provider's
   doc-comment): last-segment package→module mapping (probes 10/11),
   no stdlib/GOROOT leg (probe 09).

## Run discipline

- `cargo test -p redline-resolve --test golden_go` (picked up by
  `cargo test --workspace`). The provider is read-only over the
  checked-in corpus — no tempdir copy, no toolchain, no PTY.
- Deliberate provider/seam behavior changes are deliberate golden
  diffs: re-bless with
  `GOLDEN_BLESS=1 cargo test -p redline-resolve --test golden_go`.
  011-08 js-review P2-1: a bless run writes EVERY golden first,
  eprintlns a REWROTE / no-change line per file, then FAILS the run (a
  per-file panic would abort the loop mid-run; the end-of-test
  `assert_bless_stopped` makes an accidental bless never end green).
  Review `git diff` before committing a re-bless.
