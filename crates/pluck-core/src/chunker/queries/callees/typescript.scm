; Direct callees inside a chunk — call_expression's `function` field.
; Captures both bare-identifier calls (`foo()`) and member-expression
; calls (`db.user.findOne()`).
(call_expression
  function: (identifier) @callee)

(call_expression
  function: (member_expression) @callee)

; new Foo(...) — constructor invocations.
(new_expression
  constructor: (identifier) @callee)

(new_expression
  constructor: (member_expression) @callee)
