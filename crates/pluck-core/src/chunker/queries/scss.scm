; SCSS chunker query

; Selector rules, including nested SCSS selectors.
(rule_set
  (selectors) @module.name) @module.definition

; CSS-compatible at-rules.
(media_statement) @module.name @module.definition
(keyframes_statement) @module.name @module.definition
(supports_statement) @module.name @module.definition
(import_statement) @module.name @module.definition
(charset_statement) @module.name @module.definition
(namespace_statement) @module.name @module.definition
(at_rule) @module.name @module.definition
(use_statement) @module.name @module.definition
(forward_statement) @module.name @module.definition

; SCSS-specific at-rule-like blocks.
(mixin_statement
  name: (identifier) @module.name) @module.definition

(function_statement
  name: (identifier) @module.name) @module.definition
