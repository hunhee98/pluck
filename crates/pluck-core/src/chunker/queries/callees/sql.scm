; SQL function-body callees.
;
; PL/pgSQL bodies and SELECT expressions wrap each call in an
; `invocation` node whose first child is the object_reference of the
; called function (`lower(...)`, `now()`, `schema.fn(...)`).
(invocation
  (object_reference name: (identifier) @callee))
