; SQL chunker query.
;
; tree-sitter-sequel (derekstride/tree-sitter-sql) folds each top-level
; statement under (program (statement (...))). We capture the create_*
; nodes and alter_table directly so migration files index too.
;
; Limitation: tree-sitter-sequel does not model CREATE PROCEDURE; such
; statements trigger parse-recovery and may not produce chunks. Cover
; functions instead; procedure support waits on either an upstream
; grammar fix or a switch to a different SQL parser.
;
; Anchors (`.`) restrict each capture to the FIRST object_reference
; following the relevant keyword so trigger / function bodies (which
; nest more object_references for the table they target and the
; callees they invoke) don't yield duplicate chunks per statement.

(create_table
  (keyword_table) .
  (object_reference name: (identifier) @class.name)) @class.definition

(create_view
  (keyword_view) .
  (object_reference name: (identifier) @class.name)) @class.definition

; CREATE INDEX exposes the index name via a `column:` field on the
; outer create_index node, not via object_reference.
(create_index
  column: (identifier) @module.name) @module.definition

(create_function
  (keyword_function) .
  (object_reference name: (identifier) @function.name)) @function.definition

(create_trigger
  (keyword_trigger) .
  (object_reference name: (identifier) @function.name)) @function.definition

; Migrations: ALTER TABLE foo … is its own chunk under the targeted
; table, so a migration file surfaces the touched object even when the
; original CREATE TABLE lives elsewhere.
(alter_table
  (keyword_table) .
  (object_reference name: (identifier) @module.name)) @module.definition
