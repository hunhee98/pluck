; C++ chunker query.
;
; Extends the C surface with namespace / class / template / qualified-
; member-impl / destructor / operator overload patterns. The grammar
; carries node kinds C doesn't have:
;
; - `class_specifier`             — `class Foo { … };`
; - `namespace_definition`        — `namespace ns { … }`
; - `nested_namespace_specifier`  — `namespace a::b { … }`
; - `template_declaration`        — wraps a class / function with
;                                   `template <typename T>` prefix
; - `qualified_identifier`        — `Class::method`, `ns::fn`
; - `destructor_name`             — `~Foo`
; - `operator_name`               — `operator()`, `operator+`, …
; - `field_declaration` with
;   `function_declarator`         — member fn DECLARATION inside a
;                                   class body
;
; Member-fn declarations and out-of-class definitions are BOTH
; captured: declarations document the interface, definitions document
; the impl; agents grep either surface and benefit from chunks at
; both. Templated forms wrap the inner class / function in
; `template_declaration`; we capture both the outer (chunk content
; includes `template <…>` prefix) and the inner (clean signature) so
; the agent can pick either surface — start_bytes differ so dedup
; does not collapse them.

; ── Namespaces ────────────────────────────────────────────────────────

(namespace_definition
  name: (namespace_identifier) @module.name) @module.definition

(namespace_definition
  name: (nested_namespace_specifier) @module.name) @module.definition

; ── Classes / structs / unions / enums (standalone) ───────────────────

(class_specifier
  name: (type_identifier) @class.name) @class.definition

(struct_specifier
  name: (type_identifier) @struct.name) @struct.definition

(union_specifier
  name: (type_identifier) @struct.name) @struct.definition

(enum_specifier
  name: (type_identifier) @enum.name) @enum.definition

; ── Templated class ───────────────────────────────────────────────────

(template_declaration
  (class_specifier
    name: (type_identifier) @class.name)) @class.definition

; ── Typedef (C-style; `using Alias = …;` not yet covered) ─────────────

(type_definition
  declarator: (type_identifier) @class.name) @class.definition

; ── Free function definitions ─────────────────────────────────────────

(function_definition
  declarator: (function_declarator
    declarator: (identifier) @function.name)) @function.definition

(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @function.name))) @function.definition

; Reference-return free fn:  T &foo(…) { … }   (also `= delete` /
; `= default` member functions, which elevate to function_definition).
(function_definition
  declarator: (reference_declarator
    (function_declarator
      declarator: (identifier) @function.name))) @function.definition

; Reference-return operator (= delete / = default elevates `T &operator=…`
; from field_declaration to function_definition with a method clause).
(function_definition
  declarator: (reference_declarator
    (function_declarator
      declarator: (operator_name) @function.name))) @function.definition

; ── Templated free function ───────────────────────────────────────────

(template_declaration
  (function_definition
    declarator: (function_declarator
      declarator: (identifier) @function.name))) @function.definition

; ── Qualified function definitions (out-of-class member impl) ─────────

(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @function.name))) @function.definition

(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (destructor_name) @function.name))) @function.definition

(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (operator_name) @function.name))) @function.definition

; ── Forward declarations / prototypes (value + pointer return) ────────

(declaration
  declarator: (function_declarator
    declarator: (identifier) @function.name)) @function.definition

(declaration
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @function.name))) @function.definition

; ── In-class member declarations ──────────────────────────────────────

; Regular method:  bool verify(const std::string &) const;
(field_declaration
  declarator: (function_declarator
    declarator: (field_identifier) @function.name)) @function.definition

; Regular method with reference return:  T &method() …;
(field_declaration
  declarator: (reference_declarator
    (function_declarator
      declarator: (field_identifier) @function.name))) @function.definition

; Operator overload:  Status operator()(...) const;
(field_declaration
  declarator: (function_declarator
    declarator: (operator_name) @function.name)) @function.definition

; Operator overload with reference return:  T &operator=(const T &) …;
(field_declaration
  declarator: (reference_declarator
    (function_declarator
      declarator: (operator_name) @function.name))) @function.definition

; Destructor:  ~Foo();
(declaration
  declarator: (function_declarator
    declarator: (destructor_name) @function.name)) @function.definition

; ── Macros (same as C) ────────────────────────────────────────────────

(preproc_def
  name: (identifier) @module.name) @module.definition

(preproc_function_def
  name: (identifier) @function.name) @function.definition
