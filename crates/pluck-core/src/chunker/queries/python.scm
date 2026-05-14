; Python chunker query

; Function definitions (sync and async — async is a leading token, not a different node)
(function_definition
  name: (identifier) @function.name) @function.definition

; Class definitions
(class_definition
  name: (identifier) @class.name) @class.definition

; Decorated functions: capture the decorated_definition so decorators are
; included in the chunk range, but use the inner function name as the symbol.
(decorated_definition
  definition: (function_definition
    name: (identifier) @function.name)) @function.definition

(decorated_definition
  definition: (class_definition
    name: (identifier) @class.name)) @class.definition
