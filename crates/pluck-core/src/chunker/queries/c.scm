; C chunker query.
;
; Captures every top-level declaration that has a name worth indexing.
; Notes on the grammar:
;
; - Functions: `function_definition` carries a body; `declaration` with
;   a `function_declarator` is a forward decl (prototype). Both are
;   captured so headers + sources index symmetrically. Pointer-return
;   functions wrap the `function_declarator` in a `pointer_declarator`,
;   so each function form has both a direct and a pointer-return
;   variant pattern.
;
; - Types: `typedef struct { … } T;` produces a `type_definition` whose
;   inner struct has no `name:` — symbol comes from the typedef's
;   declarator. `typedef enum E { … } T;` has BOTH an inner name and a
;   typedef name; we emit two chunks (one for each surface) since
;   programmers grep by either, and the bodies differ at different
;   start bytes. Function-pointer typedefs (`typedef int (*cb)(…)`)
;   wrap the typedef name inside parenthesized_declarator /
;   pointer_declarator, captured by a separate variant.
;
; - Macros: object-like (`#define X 1`) becomes Module; function-like
;   (`#define CLAMP(x, lo, hi) …`) becomes Function — the call-site
;   shape is what matters for retrieval.

; Function with primitive / user-typedef'd value return.
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @function.name)) @function.definition

; Function with pointer return: T *foo() { … }
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @function.name))) @function.definition

; Forward declaration (prototype) with value return.
(declaration
  declarator: (function_declarator
    declarator: (identifier) @function.name)) @function.definition

; Forward declaration with pointer return: extern T *foo(…);
(declaration
  declarator: (pointer_declarator
    declarator: (function_declarator
      declarator: (identifier) @function.name))) @function.definition

; Typedef of a value type: typedef struct {…} T;  typedef enum E {…} T;
(type_definition
  declarator: (type_identifier) @class.name) @class.definition

; Typedef of a function pointer: typedef int (*cb)(…);
(type_definition
  declarator: (function_declarator
    declarator: (parenthesized_declarator
      (pointer_declarator
        declarator: (type_identifier) @class.name)))) @class.definition

; Standalone (non-typedef) named struct / enum / union.
(struct_specifier
  name: (type_identifier) @struct.name) @struct.definition

(enum_specifier
  name: (type_identifier) @enum.name) @enum.definition

(union_specifier
  name: (type_identifier) @struct.name) @struct.definition

; Object-like macro: #define X ...
(preproc_def
  name: (identifier) @module.name) @module.definition

; Function-like macro: #define X(...) ...
(preproc_function_def
  name: (identifier) @function.name) @function.definition
