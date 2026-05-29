; Swift direct callees.
;
; Plain:    foo(args)      → call_expression (simple_identifier)
; Method:   obj.foo(args)  → call_expression (navigation_expression
;                              suffix: (navigation_suffix
;                                suffix: (simple_identifier)))
; Chained:  a.b.foo(args)  → the innermost navigation_suffix identifier is
;                            the called method; intermediate receivers are
;                            nested navigation_expressions we don't capture.

(call_expression
  (simple_identifier) @callee)

(call_expression
  (navigation_expression
    suffix: (navigation_suffix
      suffix: (simple_identifier) @callee)))
