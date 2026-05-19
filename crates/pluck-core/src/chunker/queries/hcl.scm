; HCL chunker query.
;
; Every HCL declaration is a `(block)` opened by an `(identifier)`
; (the block type) and followed by zero, one, or two `(string_lit)`
; labels:
;   terraform { … }              ; 0 labels
;   locals { … }                 ; 0 labels
;   provider "aws" { … }         ; 1 label
;   variable "region" { … }      ; 1 label
;   data "type" "name" { … }     ; 2 labels
;   resource "type" "name" { … } ; 2 labels
;
; We capture every block uniformly as a Module chunk; the dotted
; symbol composed by `normalize_hcl_symbol` (in mod.rs) carries the
; discrimination — e.g. `resource.aws_s3_bucket.main` vs
; `variable.region` vs `terraform`. Block-type discrimination via
; `@class` / `@module` capture suffixes would require the `#eq?`
; predicate, which our QueryCursor.matches() invocation does not
; auto-filter.
;
; Nested blocks (backend / lifecycle / dynamic / provisioner inside
; another block) are captured too, so an agent searching for "backend"
; finds the nested `backend.s3` chunk inside `terraform { … }`.

(block
  (identifier) @module.name) @module.definition
