; Kotlin direct callees.
;
; Plain:    foo(args)               → call_expression (identifier)
; Receiver: bar.foo(args)           → call_expression (navigation_expression
;                                       (identifier) (identifier))
; Chained:  baz.bar.foo(args)       → call_expression (navigation_expression
;                                       (navigation_expression …) (identifier))

(call_expression
  (identifier) @callee)

(call_expression
  (navigation_expression
    (_)
    (identifier) @callee))
