; C direct callees: foo(args), ns->foo(args) via field expressions.
;
; Function-pointer calls of the form `(*fn)(args)` use a
; parenthesized_expression / pointer_expression in the `function:`
; field rather than a bare identifier; skipping those keeps the
; callee list focused on syntactically resolvable names.
(call_expression
  function: (identifier) @callee)
