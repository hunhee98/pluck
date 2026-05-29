; PHP direct callees.
;
; foo()        → function_call_expression function: (name)
; Ns\foo()     → function_call_expression function: (qualified_name (name))
; $obj->foo()  → member_call_expression name: (name)
; Foo::bar()   → scoped_call_expression name: (name)
(function_call_expression
  function: (name) @callee)

(function_call_expression
  function: (qualified_name (name) @callee))

(member_call_expression
  name: (name) @callee)

(scoped_call_expression
  name: (name) @callee)
