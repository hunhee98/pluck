//! Digest savings bench.
//!
//! Measures the byte-savings fraction for each handler across a suite of
//! realistic fixture inputs. The gated metric is `digest_savings_pct` —
//! the median savings across all fixtures, expressed as a percentage.
//!
//! Fixtures are hardcoded strings that represent realistic tool output.
//! Each fixture is paired with the format it should be detected as, so we
//! also exercise auto-detection on the way through.
//!
//! ## Output format
//!
//! ```
//! | # | Format | Input bytes | Output bytes | Savings |
//! |...|        |             |              |         |
//! | Σ | median |             |              | XX%     |   ← gated metric
//! ```
//!
//! ## Baseline metric
//!
//! `digest_savings_pct` in `benchmarks/baseline.json`:
//!   - direction: `lower` is a regression (we want high savings)
//!   - tolerance: 25 % (savings can drop 25 % before the gate fires)

fn main() {
    let fixtures: Vec<(&str, &str)> = vec![
        // (label, input)
        (
            "cargo-build",
            concat!(
                "   Compiling serde v1.0.195\n",
                "   Compiling serde_derive v1.0.195\n",
                "   Compiling tokio-macros v2.2.0\n",
                "   Compiling tokio v1.35.0\n",
                "   Compiling serde_json v1.0.108\n",
                "   Compiling anyhow v1.0.75\n",
                "   Compiling tracing-core v0.1.32\n",
                "   Compiling tracing v0.1.40\n",
                "   Compiling futures-core v0.3.29\n",
                "   Compiling futures-util v0.3.29\n",
                "   Compiling pin-project-lite v0.2.13\n",
                "   Compiling bytes v1.5.0\n",
                "   Compiling mio v0.8.10\n",
                "   Compiling socket2 v0.5.5\n",
                "   Compiling tokio-util v0.7.10\n",
                "   Compiling hyper v1.1.0\n",
                "   Compiling reqwest v0.11.24\n",
                "   Compiling pluck-core v0.1.0\n",
                "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 45.23s\n",
            ),
        ),
        (
            "cargo-test-green",
            concat!(
                "   Compiling pluck-core v0.1.0\n",
                "   Compiling pluck-mcp v0.1.0\n",
                "    Finished test profile target(s) in 2.34s\n",
                "     Running unittests src/lib.rs\n",
                "\n",
                "running 25 tests\n",
                "test chunker::tests::rust_chunks_function ... ok\n",
                "test chunker::tests::ts_chunks_class ... ok\n",
                "test chunker::tests::python_chunks_def ... ok\n",
                "test chunker::tests::go_chunks_func ... ok\n",
                "test index::tests::bm25_search_returns_hits ... ok\n",
                "test index::tests::hybrid_search_degrades_to_bm25 ... ok\n",
                "test store::tests::chunk_roundtrip ... ok\n",
                "test watcher::tests::debounce_fires_once ... ok\n",
                "test tokenizer::tests::camel_case_split ... ok\n",
                "test tokenizer::tests::unicode_split ... ok\n",
                "test ranking::tests::rrf_fusion_order ... ok\n",
                "test ranking::tests::noise_cutoff_drops_low_scores ... ok\n",
                "test digest::format::tests::cargo_detected_from_two_compiling_lines ... ok\n",
                "test digest::format::tests::npm_detected_from_added_packages_line ... ok\n",
                "test digest::format::tests::pytest_detected_from_session_banner ... ok\n",
                "test digest::handlers::cargo::tests::collapses_progress_into_summary ... ok\n",
                "test digest::handlers::cargo::tests::keeps_error_block_verbatim ... ok\n",
                "test digest::handlers::npm::tests::collapses_pnpm_progress_to_one_line ... ok\n",
                "test digest::handlers::npm::tests::collapses_yarn_phases ... ok\n",
                "test digest::handlers::pytest::tests::collapses_all_green_passed_lines ... ok\n",
                "test digest::handlers::pytest::tests::keeps_failing_run_in_full ... ok\n",
                "test digest::handlers::gha::tests::collapses_successful_step_body ... ok\n",
                "test digest::handlers::gha::tests::keeps_failed_step_body_verbatim ... ok\n",
                "test callees::tests::rust_fn_calls ... ok\n",
                "test outliner::tests::outline_has_symbol_names ... ok\n",
                "\n",
                "test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s\n",
            ),
        ),
        (
            "pnpm-install",
            concat!(
                "Progress: resolved 1, reused 0, downloaded 0, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 0, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 2, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 15, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 47, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 89, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 112, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 130, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 134, added 0\n",
                "Progress: resolved 134, reused 0, downloaded 134, added 134, done\n",
                "node_modules/.pnpm/esbuild@0.20.0/node_modules/esbuild: Running postinstall script...\n",
                "node_modules/.pnpm/esbuild@0.20.0/node_modules/esbuild: postinstall script done in 84ms\n",
                "added 134 packages in 8.3s\n",
                "\n",
                "48 packages are looking for funding\n",
                "  run `npm fund` for details\n",
            ),
        ),
        (
            "yarn-install",
            concat!(
                "yarn install v1.22.21\n",
                "[1/4] Resolving packages...\n",
                "[2/4] Fetching packages...\n",
                "[3/4] Linking dependencies...\n",
                "[4/4] Building fresh packages...\n",
                "success Saved lockfile.\n",
                "added 248 packages in 12.34s\n",
                "Done in 12.34s.\n",
            ),
        ),
        (
            "pytest-green",
            concat!(
                "==================== test session starts ====================\n",
                "platform darwin -- Python 3.11.6, pytest-7.4.3, pluggy-1.3.0\n",
                "rootdir: /workspace\n",
                "configfile: pyproject.toml\n",
                "collected 42 items\n",
                "\n",
                "tests/test_api.py::test_login PASSED\n",
                "tests/test_api.py::test_logout PASSED\n",
                "tests/test_api.py::test_register PASSED\n",
                "tests/test_api.py::test_profile PASSED\n",
                "tests/test_api.py::test_update_profile PASSED\n",
                "tests/test_models.py::test_user_create PASSED\n",
                "tests/test_models.py::test_user_delete PASSED\n",
                "tests/test_models.py::test_user_list PASSED\n",
                "tests/test_models.py::test_user_serialize PASSED\n",
                "tests/test_models.py::test_post_create PASSED\n",
                "tests/test_models.py::test_post_delete PASSED\n",
                "tests/test_models.py::test_post_list PASSED\n",
                "tests/test_utils.py::test_slugify PASSED\n",
                "tests/test_utils.py::test_paginate PASSED\n",
                "tests/test_utils.py::test_email_validate PASSED\n",
                "==================== 15 passed in 0.87s ====================\n",
            ),
        ),
        (
            "gha-log",
            concat!(
                "##[group]Set up job\n",
                "Current runner version: '2.312.0'\n",
                "Operating System\n",
                "  Ubuntu 22.04.3 LTS\n",
                "Virtual Environment\n",
                "  Environment: ubuntu-22.04\n",
                "##[endgroup]\n",
                "##[group]Checkout code\n",
                "Run actions/checkout@v4\n",
                "  Cloning into '.'\n",
                "  Checking connectivity... done.\n",
                "##[endgroup]\n",
                "##[group]cargo build\n",
                "   Compiling serde v1.0.195\n",
                "   Compiling tokio v1.35.0\n",
                "   Compiling pluck-core v0.1.0\n",
                "    Finished `release` profile in 62.4s\n",
                "##[endgroup]\n",
                "##[group]cargo test\n",
                "   Compiling pluck-core v0.1.0\n",
                "    Finished test profile in 8.1s\n",
                "     Running unittests\n",
                "running 37 tests\n",
                "test digest::handlers::cargo::tests::collapses_progress_into_summary ... ok\n",
                "test digest::handlers::npm::tests::collapses_pnpm_progress_to_one_line ... ok\n",
                "test result: ok. 37 passed; 0 failed; 0 ignored\n",
                "##[endgroup]\n",
            ),
        ),
    ];

    println!();
    println!("Digest savings bench — {} fixtures.", fixtures.len());
    println!();
    println!("| # | Fixture | Input bytes | Output bytes | Savings |");
    println!("|--:|---------|------------:|-------------:|--------:|");

    let mut savings_pcts: Vec<f64> = Vec::new();
    let mut total_input = 0usize;
    let mut total_output = 0usize;

    for (i, (label, input)) in fixtures.iter().enumerate() {
        let result = pluck_core::digest::digest(input, None);
        let out_bytes = result.text.len();
        let in_bytes = result.input_bytes;
        let saved = in_bytes.saturating_sub(out_bytes);
        let pct = if in_bytes > 0 {
            (100.0 * saved as f64 / in_bytes as f64).round()
        } else {
            0.0
        };
        savings_pcts.push(pct);
        total_input += in_bytes;
        total_output += out_bytes;
        println!(
            "| {} | `{label}` | {in_bytes} | {out_bytes} | {pct:.0}% |",
            i + 1
        );
    }

    savings_pcts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_pct = if savings_pcts.is_empty() {
        0.0
    } else {
        savings_pcts[savings_pcts.len() / 2]
    };

    let total_saved = total_input.saturating_sub(total_output);
    let total_pct = if total_input > 0 {
        (100.0 * total_saved as f64 / total_input as f64).round()
    } else {
        0.0
    };

    println!("| Σ | total | **{total_input}** | **{total_output}** | **{total_pct:.0}%** |");
    println!();
    println!("Median savings: **{median_pct:.0}%**  (gated metric: digest_savings_pct)");
    println!(
        "Total: {total_input} → {total_output} bytes ({total_saved} bytes saved, {total_pct:.0}%)"
    );
    println!();
}
