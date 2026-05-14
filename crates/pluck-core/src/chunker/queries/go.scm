; Go chunker query

; Top-level functions: `func foo() {}`
(function_declaration
  name: (identifier) @function.name) @function.definition

; Methods with receivers: `func (r *Foo) bar() {}`
(method_declaration
  name: (field_identifier) @method.name) @method.definition

; Struct types: `type Foo struct { ... }`
(type_declaration
  (type_spec
    name: (type_identifier) @struct.name
    type: (struct_type))) @struct.definition

; Interface types: `type Foo interface { ... }`
(type_declaration
  (type_spec
    name: (type_identifier) @class.name
    type: (interface_type))) @class.definition
