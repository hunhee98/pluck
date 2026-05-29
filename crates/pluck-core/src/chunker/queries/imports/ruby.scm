; Ruby has no static import statement — dependencies load via `require` /
; `require_relative`, which are ordinary method calls (already surfaced as
; callees). No file-level import extraction; this query is intentionally
; empty so the merged query compiles with an import slot present.
