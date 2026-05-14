; Rust chunker query

; Free functions and associated functions (methods inside impl blocks)
(function_item
  name: (identifier) @function.name) @function.definition

; Struct definitions
(struct_item
  name: (type_identifier) @struct.name) @struct.definition

; Enum definitions
(enum_item
  name: (type_identifier) @enum.name) @enum.definition

; Trait definitions
(trait_item
  name: (type_identifier) @trait.name) @trait.definition

; Impl blocks — use the implementing type as the symbol name.
; Handles `impl Foo {}` and `impl Trait for Foo {}` (type field = Foo).
; Generic `impl Foo<T>` falls through (type field = generic_type) — TODO.
(impl_item
  type: (type_identifier) @impl.name) @impl.definition

; Modules: `mod foo { ... }`
(mod_item
  name: (identifier) @module.name) @module.definition

; Type aliases: `type Foo = Bar;`
(type_item
  name: (type_identifier) @struct.name) @struct.definition
