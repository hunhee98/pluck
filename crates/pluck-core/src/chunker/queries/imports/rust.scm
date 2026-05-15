; Rust imports.
;
; Captures the full path argument of each `use` declaration. The
; subtree may be a scoped_identifier (`foo::bar::baz`), a use_list
; (`foo::{bar, baz}`), or a use_as_clause (`foo as f`). We emit the
; whole argument text and let post-processing parse it.
(use_declaration
  argument: (_) @import)

; `extern crate foo;` — names the crate, no path.
(extern_crate_declaration
  name: (identifier) @import)
