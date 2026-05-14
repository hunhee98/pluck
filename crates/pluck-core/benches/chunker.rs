use criterion::{black_box, criterion_group, criterion_main, Criterion};
use pluck_core::chunker::{chunk_source, Language};

fn make_ts_source(num_fns: usize) -> String {
    let mut s = String::with_capacity(num_fns * 80);
    for i in 0..num_fns {
        s.push_str(&format!(
            "async function fn_{i}(arg: string): Promise<void> {{\n  console.log(arg);\n  return;\n}}\n\n"
        ));
    }
    s
}

fn bench_chunk_small(c: &mut Criterion) {
    let src = r#"
async function processToken(token: string): Promise<boolean> {
  if (!token) return false;
  const parts = token.split(".");
  return parts.length === 3;
}
const validate = (s: string) => s.length > 0;
class Noop {
  run(): void {}
}
"#;
    c.bench_function("chunk_small (10 lines)", |b| {
        b.iter(|| chunk_source(black_box(src), Language::TypeScript).unwrap())
    });
}

fn bench_chunk_medium(c: &mut Criterion) {
    // ~500 lines: 25 fns × ~20 lines each (5 body lines + blank = ~125 fns × 4 lines)
    let src = make_ts_source(100); // 100 fns × 5 lines = 500 lines
    c.bench_function("chunk_medium (500 lines)", |b| {
        b.iter(|| chunk_source(black_box(&src), Language::TypeScript).unwrap())
    });
}

fn bench_chunk_large(c: &mut Criterion) {
    // ~5000 lines: 1000 fns × 5 lines
    let src = make_ts_source(1000);
    c.bench_function("chunk_large (5000 lines)", |b| {
        b.iter(|| chunk_source(black_box(&src), Language::TypeScript).unwrap())
    });
}

criterion_group!(
    benches,
    bench_chunk_small,
    bench_chunk_medium,
    bench_chunk_large
);
criterion_main!(benches);
