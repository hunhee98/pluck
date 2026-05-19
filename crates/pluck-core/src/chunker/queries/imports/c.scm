; C `#include` directives.
;   #include <stdio.h>      -> system_lib_string  -> "<stdio.h>"
;   #include "config.h"     -> string_literal     -> "config.h" (quotes stripped by chunker)
(preproc_include
  path: (system_lib_string) @import)

(preproc_include
  path: (string_literal) @import)
