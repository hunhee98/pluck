; CSS chunker query

; Selector rules: `.card { ... }`, `.a, .b:hover { ... }`.
(rule_set
  (selectors) @module.name) @module.definition

; Built-in at-rules: `@media`, `@keyframes`, `@supports`, `@import`, etc.
(media_statement) @module.name @module.definition
(keyframes_statement) @module.name @module.definition
(supports_statement) @module.name @module.definition
(import_statement) @module.name @module.definition
(charset_statement) @module.name @module.definition
(namespace_statement) @module.name @module.definition
(scope_statement) @module.name @module.definition

; Unknown/custom at-rules.
(at_rule) @module.name @module.definition
