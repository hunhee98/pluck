; TypeScript imports.
;
; ES `import` statements — the `source` field is a string literal.
;   import foo from "./bar"
;   import { a, b } from "./bar"
;   import * as foo from "./bar"
;   import "./side-effect"
(import_statement
  source: (string) @import)

; `require("./bar")` — CommonJS.
(call_expression
  function: (identifier) @_fn (#eq? @_fn "require")
  arguments: (arguments (string) @import))

; Dynamic `import("./bar")` — runtime.
(call_expression
  function: (import)
  arguments: (arguments (string) @import))

; `export ... from "./bar"` — re-exports also create a dep edge.
(export_statement
  source: (string) @import)
