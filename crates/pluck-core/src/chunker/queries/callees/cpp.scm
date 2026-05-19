; C++ direct callees.
;
; Plain:     foo(x)             -> call_expression function: (identifier)
; Qualified: ns::foo(x), Class::static_fn(x)
;                               -> call_expression function: (qualified_identifier ... name: (identifier))
; Method:    obj.method(x), ptr->method(x)
;                               -> call_expression function: (field_expression field: (field_identifier))

(call_expression
  function: (identifier) @callee)

(call_expression
  function: (qualified_identifier
    name: (identifier) @callee))

(call_expression
  function: (field_expression
    field: (field_identifier) @callee))
