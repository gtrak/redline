# Task: coverage/matrix rows for the landed language predicates (docs)

You are the implementation worker. Repo root is your cwd. Self-contained.

## Origin

The lang-pred lane landed node predicates + scope walks for C, Cpp,
Markdown, Bash, Toml, Json (reviewer PASS; merged). Its review flags the
STALE rows (the coverage doc predates the lane):

1. **P1-follow-up**: `docs/language-coverage.md` — C/Cpp/Toml/Json M-.
   path-shaped cells still say "degrades to the bail — node_at → None";
   the scope-walk cells say "not implemented"; the column header says
   non-[] only for Rust/JS/TS/Tsx/Python/Go; Gaps item 1 says "no node
   predicates + no scope walk" (only the provider gap stands now).
   Update all to the new truth WITH the new test names as citations
   (c_member_path_comes_back_whole, cpp_qualified_path_comes_back_whole,
   toml_table_header_dotted_key_comes_back_whole,
   json_scope_chain_is_the_enclosing_key_chain, bash_scope_chain_
   function_body, markdown_scope_chain_is_the_enclosing_headings, etc.).
   Markdown/Bash M-. path-shaped cells stay as they are (still honest
   N/A). Also fix the mirrored stale cell in `docs/provider-matrix.md`
   ("the per-language node_at/scope walks stay None for C/C++...").
2. **P2 additions to the doc** (from the review, document the corners):
   (a) JSON keys with escape sequences: scope takes the FIRST
   string_content child (`{"a\nb": 1}` → element "a") — a silent
   partial, no app-side consumer yet (note it); (b) C/C++ pointer-
   returning functions scope as the pointer declarator text (`int *f()`
   → "*f") — deliberate simplification, document; (c) TOML
   `[[array.table]]` headers contribute no scope element — document;
   (d) the app-side whole-path upgrade (dotted_path_container) does not
   yet enumerate C/TOML containers — bare-segment M-. stands (note the
   boundary).

## Constraints

- Docs only (the two docs files). NO code changes. Budget ~15 tool
  calls. Commit to main. Report per-item.
