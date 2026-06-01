; GraphQL schema (SDL) chunker query.
;
; Every type-system definition carries its (name) as a direct named
; child. Kind mapping:
;   type   -> Class    interface -> Class    input -> Class
;   enum   -> Enum
;   scalar / union / directive -> Module
;   query / mutation / subscription (operation), fragment -> Method
;
; `#` line comments are the doc-comment surface (see clean_line_doc).
; GraphQL `"""..."""` / "..." description strings are a separate node, not
; line comments, so they are not captured as doc yet (future work).

(object_type_definition
  (name) @class.name) @class.definition

(interface_type_definition
  (name) @class.name) @class.definition

(input_object_type_definition
  (name) @class.name) @class.definition

(enum_type_definition
  (name) @enum.name) @enum.definition

(scalar_type_definition
  (name) @module.name) @module.definition

(union_type_definition
  (name) @module.name) @module.definition

(directive_definition
  (name) @module.name) @module.definition

; Executable documents: named operations and fragments.
(operation_definition
  (name) @method.name) @method.definition

(fragment_definition
  (fragment_name) @method.name) @method.definition
