; Markdown / MDX chunker query

; Heading sections: `# Title` plus the content nested under that heading.
(section
  (atx_heading
    heading_content: (inline) @module.name)) @module.definition

(setext_heading
  heading_content: (paragraph) @module.name) @module.definition

; Fenced code blocks are independently retrievable snippets inside docs.
(fenced_code_block) @module.name @module.definition
