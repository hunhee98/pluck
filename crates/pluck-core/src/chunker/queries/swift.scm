; Swift chunker query.
;
; tree-sitter-swift folds class / struct / enum / extension / actor into a
; single class_declaration (discriminated by a declaration_kind field we do
; not split on), so all map to Class — matching the Kotlin/Java precedent
; where interface / record / enum-like decls share the @class capture.
; protocol_declaration is separate but also maps to Class (Java treats
; interface the same way). Functions, initializers, and deinitializers map
; to Method (Kotlin maps every function the same way).

(class_declaration
  name: (type_identifier) @class.name) @class.definition

(protocol_declaration
  name: (type_identifier) @class.name) @class.definition

; `func name(…)` — top-level, member, and extension functions.
(function_declaration
  name: (simple_identifier) @method.name) @method.definition

; `init(…)` and `deinit` — the keyword itself is the symbol name.
(init_declaration
  "init" @method.name) @method.definition

(deinit_declaration
  "deinit" @method.name) @method.definition
