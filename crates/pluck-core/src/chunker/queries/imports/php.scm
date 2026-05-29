; PHP imports.
;
;   use Foo\Bar;
;   use Foo\Bar as Baz;     (alias is a sibling, not captured)
;   use function Foo\bar;   (the `function`/`const` kind keyword is a sibling)
; The imported symbol is the qualified_name inside each use clause.
(namespace_use_clause
  (qualified_name) @import)
