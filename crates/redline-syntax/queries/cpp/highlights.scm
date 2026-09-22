; redline's C++ highlight query — a SUPERSET of the pinned
; tree-sitter-cpp 0.23.4 upstream `queries/highlights.scm` (the
; upstream patterns are kept verbatim, marked "upstream"), extended
; with the patterns the grammar supports but upstream omits: variables,
; members, typedef/enum/namespace names, literals, escapes, operators,
; punctuation, and the full keyword list.
;
; Every node and field name was verified against the pinned crate's
; `src/node-types.json`; every anonymous token against the grammar's
; token set (the tree-sitter query compiler rejects unknown node kinds
; at load, but a *wrong* name on a plausible node silently never
; matches — the per-language load smoke test pins the former only).
;
; Ordering rule: broad captures come FIRST so that on a node captured
; by several patterns the most specific (latest) pattern wins — the
; highlighter's end-stack is LIFO in query order, and the reuse
; pipeline's coalescing implements the same rule.

; ── Base identifiers (the broad matches; specific patterns below win) ──

(identifier) @variable
(field_identifier) @property

; ── Operators and punctuation (BEFORE the types section, so the
;    type-adjacent captures below win on shared nodes) ─────────────

[
  "!"
  "!="
  "%"
  "%="
  "&"
  "&&"
  "&="
  "*"
  "*="
  "+"
  "++"
  "+="
  "-"
  "--"
  "-="
  "/"
  "/="
  "<"
  "<="
  "<<"
  "<<="
  "<=>"
  "="
  "=="
  ">"
  ">="
  ">>"
  ">>="
  "?"
  "^"
  "^="
  "|"
  "|="
  "||"
  "~"
  "->"
  "->*"
] @operator

[
  ","
  ";"
  "."
  ":"
  "::"
  "()"
  "[]"
] @punctuation.delimiter

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
  "<"
  ">"
] @punctuation.bracket

; ── Types ─────────────────────────────────────────────────────────

(type_identifier) @type
(primitive_type) @type.builtin
(sized_type_specifier) @type.builtin
(qualified_identifier) @type
(namespace_identifier) @namespace
(auto) @type
(type_definition (identifier) @type)

; ── Namespaces and using ─────────────────────────────────────────

(namespace_definition name: (namespace_identifier) @namespace)

; ── Functions (upstream, kept verbatim) ───────────────────────────

(call_expression
  function: (qualified_identifier
    name: (identifier) @function))

(template_function
  name: (identifier) @function)

(template_method
  name: (field_identifier) @function)

(function_declarator
  declarator: (qualified_identifier
    name: (identifier) @function))

(function_declarator
  declarator: (field_identifier) @function)

; ── Functions (extensions) ────────────────────────────────────────

(call_expression function: (identifier) @function)
(call_expression
  function: (field_expression field: (field_identifier) @function))
(operator_name) @function

; ── Types (upstream, kept verbatim) ───────────────────────────────

((namespace_identifier) @type
 (#match? @type "^[A-Z]"))

; ── Constants and builtins (upstream + extensions) ────────────────

(this) @variable.builtin
(null "nullptr" @constant)

[(true) (false)] @constant.builtin

(enumerator name: (identifier) @constant)

; ── Literals ──────────────────────────────────────────────────────

(comment) @comment

(string_literal) @string
(char_literal) @string
(raw_string_literal) @string
(escape_sequence) @escape
(number_literal) @number

; ── Preprocessor ──────────────────────────────────────────────────

[
  "#include"
  "#define"
  "#if"
  "#ifdef"
  "#ifndef"
  "#elif"
  "#elifdef"
  "#elifndef"
  "#else"
  "#endif"
] @keyword

(system_lib_string) @string
(preproc_def name: (identifier) @function)

; ── Keywords (upstream list + the remaining statement keywords) ───

[
 "break"
 "catch"
 "class"
 "co_await"
 "co_return"
 "co_yield"
 "constexpr"
 "constinit"
 "consteval"
 "const"
 "continue"
 "decltype"
 "default"
 "delete"
 "do"
 "else"
 "enum"
 "explicit"
 "extern"
 "final"
 "for"
 "friend"
 "goto"
 "if"
 "inline"
 "long"
 "mutable"
 "namespace"
 "new"
 "noexcept"
 "operator"
 "override"
 "private"
 "protected"
 "public"
 "register"
 "requires"
 "return"
 "short"
 "signed"
 "sizeof"
 "static"
 "static_assert"
 "struct"
 "switch"
 "template"
 "thread_local"
 "throw"
 "try"
 "typedef"
 "typename"
 "union"
 "unsigned"
 "using"
 "virtual"
 "volatile"
 "while"
] @keyword
