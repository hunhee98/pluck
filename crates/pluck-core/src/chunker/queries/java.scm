; Java chunker query

; Top-level and nested class-like declarations.
(class_declaration
  name: (identifier) @class.name) @class.definition

(interface_declaration
  name: (identifier) @class.name) @class.definition

(record_declaration
  name: (identifier) @class.name) @class.definition

(annotation_type_declaration
  name: (identifier) @class.name) @class.definition

(enum_declaration
  name: (identifier) @enum.name) @enum.definition

; Methods and constructors.
(method_declaration
  name: (identifier) @method.name) @method.definition

(constructor_declaration
  name: (identifier) @method.name) @method.definition

(compact_constructor_declaration
  name: (identifier) @method.name) @method.definition
