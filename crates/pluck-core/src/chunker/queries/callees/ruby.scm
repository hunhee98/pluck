; Ruby direct callees.
;
; Plain:   foo(args) / foo     → call method: (identifier)
; Method:  obj.foo(args)       → call receiver: …, method: (identifier)
; Both forms expose the called name as the `method:` field. `require` /
; `require_relative` are ordinary calls and surface here as callees.
(call
  method: (identifier) @callee)
