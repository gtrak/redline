# Task: redline-resolve — Go provider (parallel lane, logic-level tests)

You are the implementation worker. Repo root is your cwd.

**Scope fence (absolute): you own exactly ONE file:
`crates/redline-resolve/src/providers/go_provider.rs`.** Other providers
(js_provider.rs, python_provider.rs) belong to parallel lanes — never
touch them, never touch lib.rs, Cargo.toml, src/, or tools/. Your module
is pre-declared; your `#[cfg(test)] mod tests` lives inside your file.

Context: `crates/redline-resolve` is an app-free resolver crate. Read
`src/lib.rs` (ToolingProvider, SymbolContext, ResolvedSource,
crate_from_symbol) and `src/cargo.rs` (the CargoProvider — the shape to
mirror). Operator directive sanctions fetch-on-demand. **Toolchain note:
there is NO `go` binary on this machine** — so your provider ships with
LOGIC-LEVEL tests (fixture layouts, metadata parsing against recorded
command output); every test that would need the `go` binary is `#[ignore]`
with a doc-comment naming the requirement. This is the established
pattern (src/perf.rs harness).

## What to build (go_provider.rs, name "go", languages ["go"])

Resolution for a SymbolContext on a Go module project:
1. Package from the symbol: Go uses dot qualification — `fmt.Println` →
   package `fmt`; the IMPORT context maps package-name → module path
   (from the file's imports), which the app's tree-sitter layer supplies
   later; for now accept the symbol's first dot-segment as the package
   name and document the limitation (mirror cargo's bare-symbol rule for
   no-dot symbols).
2. Module → source dir: parse the workspace's `go.mod` (require block)
   and `go.sum` (module→version lines) with a small hand parser (no
   deps): `github.com/x/y v1.2.3`. Module cache dir: `go env GOMODCACHE`
   (needs the binary → #[ignore] live test) with the DEFAULT fallback
   `$HOME/go/pkg/mod` (logic-testable); module dir =
   `<cache>/<module>@<version>` with the case-encoding rule (`!` escapes
   for uppercase — document and implement: uppercase letters are escaped
   as `!` + lowercase).
3. Missing cache dir → `go mod download <module>` (fetch-on-demand,
   sanctioned) → re-locate (#[ignore] live).
4. Item locate in the package dir: `func X`, `func (r R) X`,
   `type X struct/interface`, `const X`, `var X` regexes (word-boundary,
   reject references/comments), best-effort line.
5. ResolvedSource: file, source_root = module dir, external=true (module
   cache) — workspace-local packages (in go.mod's own module path)
   external=false with the workspace as root.
6. Failure modes: no go.mod in the workspace → Err; module not required →
   Err naming it; version missing from go.sum → Err; item not found → Err
   naming package + dir.

## Tests (inside your file)

- go.mod/go.sum hand-parser: require blocks, replace directives (local
  path replace → external=false, source_root = the replaced path —
  implement this, it's how workspaces use local forks), exclude/ignore
  blocks tolerated.
- case-encoding: `github.com/USER/Repo` → `github.com/!u!s!e!r/!r!e!p!o`
  (verify the exact rule against recorded module-cache layouts from real
  go docs knowledge; state your source).
- item regexes: real Go definitions, rejects `x := X`, `// func X`,
  `func (x) name` receiver noise as applicable.
- version-selection: multiple go.sum lines (v1.2.3 + /go.mod hash lines)
  → pick the module version correctly.
- #[ignore] live: `go env GOMODCACHE` + fetch E2E (doc-comment: requires
  go toolchain).

## Verification

- `cargo test -p redline-resolve` — all green (your tests + pre-existing;
  you may NOT edit other files' tests); #[ignore]d live tests counted.
- `cargo clippy -p redline-resolve --all-targets -- -D warnings` clean.
- Report: design, parser notes, gate counts, deviations, gaps (live path
  untestable here — how the app lane should verify it later).
