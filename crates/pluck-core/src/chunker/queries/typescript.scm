; Based on helix-editor/helix runtime/queries/ecma/textobjects.scm
; and runtime/queries/_typescript/textobjects.scm

; ── Functions ────────────────────────────────────────────────────────────────

; Named function declaration: function foo() {}
; Async variant:             async function foo() {}
(function_declaration
  name: (identifier) @function.name) @function.definition

; Generator function declaration: function* foo() {}
; Async generator:              async function* foo() {}
(generator_function_declaration
  name: (identifier) @function.name) @function.definition

; Arrow function bound to variable: const foo = () => {}
; const foo = async () => {}
(lexical_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function))) @function.definition

; ── Methods ──────────────────────────────────────────────────────────────────

; Class method: foo() {} / async foo() {} / get foo() {}
(method_definition
  name: (property_identifier) @method.name) @method.definition

; ── Classes ──────────────────────────────────────────────────────────────────

; class Foo {}  /  abstract class Foo {}
(class_declaration
  name: (type_identifier) @class.name) @class.definition

; ── Interfaces ───────────────────────────────────────────────────────────────

; interface Foo {}
(interface_declaration
  name: (type_identifier) @class.name) @class.definition

; ── Enums ────────────────────────────────────────────────────────────────────

; enum Direction {}  /  const enum Direction {}
(enum_declaration
  name: (identifier) @enum.name) @enum.definition
