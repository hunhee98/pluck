; Python imports.
;
; `import foo` / `import foo.bar` / `import foo as f` / `import foo, bar`
; The name is a dotted_name or aliased_import.
(import_statement
  name: (_) @import)

; `from foo import bar` / `from foo.bar import baz` / `from . import foo`
; module_name is what we want for the deps edge — `bar`/`baz` are
; symbol imports within the resolved module.
(import_from_statement
  module_name: (_) @import)
