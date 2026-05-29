; Ruby chunker query.
;
; class / module names may be a bare constant or a scope_resolution
; (`Foo::Bar`); we capture the bare-constant form. method and
; singleton_method (`def self.x`) both map to Method; module maps to
; Module (ChunkKind::Module, matching Rust's mod_item), class to Class.

(class
  name: (constant) @class.name) @class.definition

(module
  name: (constant) @module.name) @module.definition

; `def name` — instance methods and top-level (main-object) methods.
(method
  name: (_) @method.name) @method.definition

; `def self.name` / `def Receiver.name` — singleton (class/module) methods.
(singleton_method
  name: (_) @method.name) @method.definition
