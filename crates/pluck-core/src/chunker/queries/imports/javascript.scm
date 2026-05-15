; JavaScript imports — same shape as TypeScript.
(import_statement
  source: (string) @import)

(call_expression
  function: (identifier) @_fn (#eq? @_fn "require")
  arguments: (arguments (string) @import))

(call_expression
  function: (import)
  arguments: (arguments (string) @import))

(export_statement
  source: (string) @import)
