; HTML chunker query

; Normal elements: <main>...</main>, <section>...</section>, custom elements.
(element
  (start_tag
    (tag_name) @module.name)) @module.definition

; Self-closing component-ish elements: <app-card />
(element
  (self_closing_tag
    (tag_name) @module.name)) @module.definition

; Script and style blocks are distinct node kinds in tree-sitter-html.
(script_element
  (start_tag
    (tag_name) @module.name)) @module.definition

(style_element
  (start_tag
    (tag_name) @module.name)) @module.definition
