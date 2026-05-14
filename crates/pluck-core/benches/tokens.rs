//! Token-savings measurement: outline vs raw cat-equivalent output.
//!
//! Not a Criterion latency bench. Single-pass measurement that emits a
//! markdown table of token counts using the cl100k_base BPE (the tokenizer
//! family Claude / GPT-4 use). Run via `cargo bench --bench tokens`.

use pluck_core::chunker::Language;
use pluck_core::outliner::{outline_source, render};
use tiktoken_rs::cl100k_base;

/// Generate a TS source with `n` functions, each with a realistic ~20-line body.
fn gen_realistic_fns(n: usize) -> String {
    let mut s = String::with_capacity(n * 600);
    s.push_str(
        "import { Database } from \"./db\";\nimport { Logger } from \"./logger\";\nimport { metrics } from \"./metrics\";\n\n",
    );
    for i in 0..n {
        s.push_str(&format!(
            "export async function handle_{i}(req: RequestContext, db: Database): Promise<HandlerResult> {{\n  const start = Date.now();\n  const logger = Logger.forRequest(req.id);\n  logger.debug(`handle_{i} entry`);\n\n  const session = await db.sessions.findById(req.sessionId);\n  if (!session) {{\n    logger.warn(\"missing session\");\n    return {{ ok: false, error: \"SESSION_NOT_FOUND\" }};\n  }}\n  if (session.expiresAt < Date.now()) {{\n    return {{ ok: false, error: \"SESSION_EXPIRED\" }};\n  }}\n\n  const user = await db.users.findById(session.userId);\n  if (!user || user.disabled) {{\n    return {{ ok: false, error: \"USER_DISABLED\" }};\n  }}\n\n  metrics.histogram(\"handle_{i}.latency_ms\", Date.now() - start);\n  return {{ ok: true, user, session }};\n}}\n\n"
        ));
    }
    s
}

/// Class with `n_methods` realistic methods.
fn gen_realistic_class(n_methods: usize) -> String {
    let mut s = String::from(
        "import { Logger } from \"./logger\";\nimport { Database } from \"./db\";\n\nexport class AuthService {\n  private db: Database;\n  private logger: Logger;\n\n  constructor(db: Database, logger: Logger) {\n    this.db = db;\n    this.logger = logger;\n  }\n\n",
    );
    for i in 0..n_methods {
        s.push_str(&format!(
            "  async method_{i}(arg: string, opts?: MethodOpts): Promise<MethodResult<{i}>> {{\n    this.logger.debug(`method_{i} arg=${{arg}}`);\n    const validated = await this.validate(arg);\n    if (!validated.ok) {{\n      this.logger.warn(`method_{i} validation failed`);\n      return {{ ok: false, error: validated.error }};\n    }}\n    const rows = await this.db.query<Row>(`SELECT * FROM t_{i} WHERE k = ?`, [arg]);\n    if (rows.length === 0) {{\n      return {{ ok: false, error: \"NOT_FOUND\" }};\n    }}\n    const result = rows[0];\n    await this.db.audit.write({{ op: \"method_{i}\", arg, at: Date.now() }});\n    return {{ ok: true, value: result }};\n  }}\n\n"
        ));
    }
    s.push_str("}\n");
    s
}

fn main() {
    let bpe = cl100k_base().expect("load cl100k_base");
    let count = |s: &str| bpe.encode_with_special_tokens(s).len();

    let fixtures: Vec<(&str, String)> = vec![
        ("tiny (raw mode, < 100 lines)", {
            let mut s = String::new();
            for i in 0..5 {
                s.push_str(&format!(
                    "function tiny_{i}(): number {{ return {i}; }}\n\n"
                ));
            }
            s
        }),
        ("medium realistic (5 fns, ~120 lines)", gen_realistic_fns(5)),
        (
            "large realistic (25 fns, ~600 lines)",
            gen_realistic_fns(25),
        ),
        (
            "xl realistic (100 fns, ~2400 lines)",
            gen_realistic_fns(100),
        ),
        ("class (1 class + 10 methods)", gen_realistic_class(10)),
        ("class (1 class + 50 methods)", gen_realistic_class(50)),
    ];

    println!();
    println!("| Scenario | Lines | cat tokens | pluck.read tokens | savings |");
    println!("|----------|------:|-----------:|------------------:|--------:|");

    for (name, src) in &fixtures {
        let lines = if src.is_empty() {
            0
        } else {
            let nl = src.bytes().filter(|&b| b == b'\n').count();
            if src.ends_with('\n') {
                nl
            } else {
                nl + 1
            }
        };
        let o = outline_source(src, Some(Language::TypeScript), "fixture.ts");
        let rendered = render(&o);
        let cat_tokens = count(src);
        let pluck_tokens = count(&rendered);
        let pct = if cat_tokens > 0 {
            (100.0 * (cat_tokens as f64 - pluck_tokens as f64) / cat_tokens as f64).round() as i64
        } else {
            0
        };
        println!("| {name} | {lines} | {cat_tokens} | {pluck_tokens} | {pct}% |");
    }
    println!();
}
