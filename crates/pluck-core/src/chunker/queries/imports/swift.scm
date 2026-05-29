; Swift imports.
;
;   import Foo
;   import Foo.Bar           (the dotted path is one identifier node)
;   @testable import Foo     (the attribute/modifier is a sibling, not captured)
;   import class Foo.Bar     (the kind keyword is a sibling, not captured)
(import_declaration
  (identifier) @import)
