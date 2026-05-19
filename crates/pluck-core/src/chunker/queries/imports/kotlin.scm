; Kotlin imports.
;
;   import foo.bar.Baz
;   import foo.bar.*           (the wildcard is sibling text, not captured)
;   import foo.bar.Baz as Qux  (the alias is a sibling identifier, not captured)
(import
  (qualified_identifier) @import)
