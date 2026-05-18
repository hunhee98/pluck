; Kotlin chunker query.
;
; tree-sitter-kotlin-ng folds class / interface / enum / data class /
; sealed class into a single class_declaration; the discriminator is a
; sibling modifiers / keyword node we don't capture for the symbol. All
; variants resolve to Class kind at extraction time.

(class_declaration
  name: (identifier) @class.name) @class.definition

; `object Foo { … }` and companion objects.
(object_declaration
  name: (identifier) @class.name) @class.definition

; `fun name(…)` — top-level, member, and extension functions.
(function_declaration
  name: (identifier) @method.name) @method.definition
