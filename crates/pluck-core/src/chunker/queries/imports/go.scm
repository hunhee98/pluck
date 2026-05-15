; Go imports.
;
;   import "fmt"
;   import ( "fmt"; "os" )
;   import alias "fmt"
; Each import_spec has a `path` field that is an interpreted_string_literal.
(import_spec
  path: (_) @import)
