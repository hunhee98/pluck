; Rust direct callees.
;
; Method calls `obj.foo(...)` parse as call_expression where the function
; field is a `field_expression`, so the field_expression pattern below
; covers them. There is no `method_call_expression` node in this grammar.
(call_expression
  function: (identifier) @callee)

(call_expression
  function: (scoped_identifier) @callee)

(call_expression
  function: (field_expression) @callee)

; Macro invocations — `println!`, `vec!`, etc.
(macro_invocation
  macro: (identifier) @callee)

(macro_invocation
  macro: (scoped_identifier) @callee)
