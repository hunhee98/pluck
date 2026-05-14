; JavaScript chunker query
; Note: class name in JS grammar uses `identifier`, not `type_identifier` as in TS.

; Named function declaration (sync/async)
(function_declaration
  name: (identifier) @function.name) @function.definition

; Generator function declaration
(generator_function_declaration
  name: (identifier) @function.name) @function.definition

; Arrow function bound to a const/let/var: `const foo = () => {}`
(lexical_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function))) @function.definition

(variable_declaration
  (variable_declarator
    name: (identifier) @function.name
    value: (arrow_function))) @function.definition

; Class method
(method_definition
  name: (property_identifier) @method.name) @method.definition

; Class declaration
(class_declaration
  name: (identifier) @class.name) @class.definition
