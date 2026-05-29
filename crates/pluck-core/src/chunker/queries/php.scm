; PHP chunker query.
;
; class -> Class, interface -> Class (no Interface kind; Java precedent),
; trait -> Trait (ChunkKind::Trait, Rust precedent), enum -> Enum,
; namespace -> Module, top-level function -> Function, method -> Method.
; PHP grammar uses a bare (name) node for every declaration name and a
; (namespace_name) for namespaces.

(namespace_definition
  name: (namespace_name) @module.name) @module.definition

(class_declaration
  name: (name) @class.name) @class.definition

(interface_declaration
  name: (name) @class.name) @class.definition

(trait_declaration
  name: (name) @trait.name) @trait.definition

(enum_declaration
  name: (name) @enum.name) @enum.definition

; Top-level / namespaced functions.
(function_definition
  name: (name) @function.name) @function.definition

; Methods inside a class / interface / trait body.
(method_declaration
  name: (name) @method.name) @method.definition
