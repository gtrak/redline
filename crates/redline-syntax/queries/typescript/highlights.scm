; redline's TypeScript/TSX highlight query — a SUPERSET of the pinned
; tree-sitter-typescript 0.23.2 upstream `queries/highlights.scm`
; (the upstream patterns are kept verbatim, marked "upstream"), extended
; with the patterns the grammar supports but upstream omits: variables,
; properties, function/method definitions and calls, constructors,
; imports/namespaces, labels, literals, escapes, tokens, and the full
; keyword list.
;
; Every node and field name was verified against the pinned crate's
; `typescript/src/node-types.json` AND `tsx/src/node-types.json`
; (this query is shared by both grammars — it may only use node kinds
; present in BOTH).
;
; Ordering rule: broad captures come FIRST so that on a node captured by
; several patterns the most specific (latest) pattern wins — the
; highlighter's end-stack is LIFO in query order, and the reuse
; pipeline's coalescing implements the same rule.

; ── Base identifiers (the broad matches; specific patterns below win) ──

(identifier) @variable

(property_identifier) @property
(shorthand_property_identifier) @property
(shorthand_property_identifier_pattern) @variable
(private_property_identifier) @property
(statement_identifier) @label
(nested_identifier) @namespace

; ── Parameters (upstream) ─────────────────────────────────────────

(required_parameter (identifier) @variable.parameter)
(optional_parameter (identifier) @variable.parameter)

; ── Function and method definitions ───────────────────────────────

(function_declaration name: (identifier) @function)
(function_expression name: (identifier) @function)
(function_signature name: (identifier) @function)
(method_definition name: (property_identifier) @function)

(variable_declarator
  name: (identifier) @function
  value: [(function_expression) (arrow_function)])

(pair
  key: (property_identifier) @function
  value: [(function_expression) (arrow_function)])

(assignment_expression
  left: (identifier) @function
  right: [(function_expression) (arrow_function)])

(assignment_expression
  left: (member_expression property: (property_identifier) @function)
  right: [(function_expression) (arrow_function)])

; ── Function and method calls ─────────────────────────────────────

(call_expression function: (identifier) @function)
(call_expression
  function: (member_expression property: (property_identifier) @function))

(new_expression constructor: (_) @constructor)

; ── Imports, exports, and namespaces ──────────────────────────────

(import_specifier name: (identifier) @function)
(import_specifier alias: (identifier) @function)
(namespace_import (identifier) @function)
(internal_module name: (identifier) @namespace)

; ── Operators and punctuation (BEFORE the types section, so the
;    type_arguments bracket capture below wins over the < / >
;    operator captures) ────────────────────────────────────────────

[";" (optional_chain) "." ","] @punctuation.delimiter

[
  "-"
  "--"
  "-="
  "+"
  "++"
  "+="
  "*"
  "*="
  "**"
  "**="
  "/"
  "/="
  "%"
  "%="
  "<"
  "<="
  "<<"
  "<<="
  "="
  "=="
  "==="
  "!"
  "!="
  "!=="
  "=>"
  ">"
  ">="
  ">>"
  ">>="
  ">>>"
  ">>>="
  "~"
  "^"
  "&"
  "|"
  "^="
  "&="
  "|="
  "&&"
  "||"
  "??"
  "&&="
  "||="
  "??="
  "?."
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

; ── Types (upstream) ──────────────────────────────────────────────

(type_identifier) @type
(predefined_type) @type.builtin

(type_arguments
  "<" @punctuation.bracket
  ">" @punctuation.bracket)

(enum_declaration name: (identifier) @type)

; ── Special identifiers ───────────────────────────────────────────
; (the A-Z type match sits AFTER the A-Z constructor match so a bare
;  capitalized identifier in value position reads as a type, matching
;  the upstream intent; the ALL_CAPS constant match sits after both so
;  it wins on `MAX_SIZE`-style names)

((identifier) @constructor
 (#match? @constructor "^[A-Z]"))

((identifier) @type
 (#match? @type "^[A-Z]"))

([(identifier) (shorthand_property_identifier) (shorthand_property_identifier_pattern)] @constant
 (#match? @constant "^[A-Z_][A-Z\\d_]+$"))

((identifier) @variable.builtin
 (#match? @variable.builtin "^(arguments|module|console|window|document)$")
 (#is-not? local))

(this) @variable.builtin
(super) @variable.builtin

[(true) (false) (null) (undefined)] @constant.builtin

; ── Literals ──────────────────────────────────────────────────────

(comment) @comment

[(string) (template_string)] @string
(escape_sequence) @escape

(regex) @string.special
(number) @number

; ── Template substitutions (the embedded expression content) ──────

(template_substitution
  "${" @punctuation.special
  "}" @punctuation.special) @embedded

; ── Keywords (upstream TS list + the shared JS-era list) ──────────

[
  "abstract"
  "as"
  "async"
  "await"
  "break"
  "case"
  "catch"
  "class"
  "const"
  "continue"
  "debugger"
  "declare"
  "default"
  "delete"
  "do"
  "else"
  "enum"
  "export"
  "extends"
  "finally"
  "for"
  "from"
  "function"
  "get"
  "if"
  "implements"
  "import"
  "in"
  "infer"
  "instanceof"
  "interface"
  "is"
  "keyof"
  "let"
  "module"
  "namespace"
  "new"
  "of"
  "override"
  "private"
  "protected"
  "public"
  "readonly"
  "return"
  "satisfies"
  "set"
  "static"
  "switch"
  "throw"
  "try"
  "type"
  "typeof"
  "using"
  "var"
  "void"
  "while"
  "with"
  "yield"
] @keyword
