; Svelte single-file-component chunker query.
;
; tree-sitter-svelte-ng reuses the HTML element grammar, so this mirrors
; html.scm: top-level elements form the markup skeleton, and <script> /
; <style> blocks become dedicated chunks so the agent can pull just the
; behaviour or styling slice. Every chunk needs a @*.name capture or the
; chunker has no symbol to key on and drops it.
;
; The JS/TS inside <script> is raw_text — this grammar does not parse it
; into symbols, so function-level chunking and callee/import extraction
; would need a JS injection pass (future work).

(element
  (start_tag
    (tag_name) @module.name)) @module.definition

(script_element
  (start_tag
    (tag_name) @module.name)) @module.definition

(style_element
  (start_tag
    (tag_name) @module.name)) @module.definition
