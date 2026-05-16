; Java direct callees.

; methodName(...) and receiver.methodName(...).
(method_invocation
  name: (identifier) @callee)

; new Type(...).
(object_creation_expression
  type: (type_identifier) @callee)

(object_creation_expression
  type: (scoped_type_identifier) @callee)

(object_creation_expression
  type: (generic_type
    [
      (type_identifier)
      (scoped_type_identifier)
    ] @callee))
