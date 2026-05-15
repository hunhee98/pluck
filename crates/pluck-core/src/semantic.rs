//! Static-embedding encoder.
//!
//! Loads a model2vec-style model — `tokenizer.json` + a `model.safetensors`
//! containing one `embeddings` tensor of shape `[vocab_size, dim]`,
//! `f32` — and runs inference by lookup + mean-pool. No matrix
//! multiplications, no ONNX, no GPU. The whole `encode` call is
//! O(n_tokens × dim) with great cache locality.
//!
//! Today: `potion-code-16M` — code-retrieval-specialized model2vec
//! distilled from `nomic-ai/CodeRankEmbed` on the CornStack code
//! corpus. Smaller than the general-purpose `potion-base-32M` but
//! tuned for code; NDCG@10 ≈ 0.85 on the standard retrieval
//! benchmarks, sub-millisecond encode on CPU.
//!
//! Fetching: [`StaticEncoder::load_or_fetch`] pulls the model from
//! Hugging Face the first time and caches under `~/.pluck/models/<id>/`
//! (override with `PLUCK_HOME`). Subsequent calls are local file reads.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use safetensors::SafeTensors;
use tokenizers::Tokenizer;

use crate::store::pluck_root;

/// Default Hugging Face model id pluck embeds with.
pub const DEFAULT_MODEL_ID: &str = "minishlab/potion-code-16M";

/// Model id selected for this process.
///
/// `DEFAULT_MODEL_ID` remains the stable default; `PLUCK_EMBED_MODEL`
/// is an escape hatch for quality experiments such as retrieval-tuned
/// model2vec variants.
pub fn selected_model_id() -> String {
    std::env::var("PLUCK_EMBED_MODEL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string())
}

/// Static (lookup-based) embedding encoder.
///
/// Holds the embedding matrix in memory as a flat `Vec<f32>` (row-major,
/// `vocab_size × dim`). `encode` borrows into it without allocating per
/// token — the only allocation per call is the output `Vec<f32>` of size
/// `dim`.
pub struct StaticEncoder {
    tokenizer: Tokenizer,
    embeddings: Vec<f32>,
    vocab_size: usize,
    dim: usize,
}

impl StaticEncoder {
    /// Load from an on-disk model directory containing `tokenizer.json` and
    /// `model.safetensors`. The safetensors file must expose a single
    /// `embeddings` tensor of dtype f32.
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let tok_path = dir.join("tokenizer.json");
        let st_path = dir.join("model.safetensors");
        let tokenizer = Tokenizer::from_file(&tok_path)
            .map_err(|e| anyhow!("load tokenizer at {tok_path:?}: {e}"))?;

        let bytes =
            std::fs::read(&st_path).with_context(|| format!("read safetensors {st_path:?}"))?;
        let st = SafeTensors::deserialize(&bytes).context("deserialize safetensors")?;

        // model2vec packages name the tensor `embeddings` — accept that
        // plus a couple of common aliases just in case.
        let name = ["embeddings", "embedding", "weight", "embeddings.weight"]
            .iter()
            .find(|n| st.tensor(n).is_ok())
            .ok_or_else(|| {
                anyhow!(
                    "safetensors at {st_path:?} has no `embeddings` tensor; \
                     found tensors: {:?}",
                    st.names()
                )
            })?;
        let tensor = st.tensor(name).unwrap();

        if tensor.dtype() != safetensors::Dtype::F32 {
            return Err(anyhow!("expected f32 embeddings, got {:?}", tensor.dtype()));
        }
        let shape = tensor.shape();
        if shape.len() != 2 {
            return Err(anyhow!("expected 2-D embeddings, got shape {shape:?}"));
        }
        let (vocab_size, dim) = (shape[0], shape[1]);

        let raw = tensor.data();
        let mut embeddings = Vec::with_capacity(vocab_size * dim);
        for chunk in raw.chunks_exact(4) {
            embeddings.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }

        Ok(Self {
            tokenizer,
            embeddings,
            vocab_size,
            dim,
        })
    }

    /// Convenience: fetch the model from Hugging Face into the pluck
    /// model cache (default `~/.pluck/models/<id>/`) and load it. The
    /// first call performs the download; subsequent calls reuse the
    /// cached files.
    pub fn load_or_fetch(model_id: &str) -> Result<Self> {
        let dir = ensure_model_dir(model_id)?;
        Self::load_from_dir(&dir)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    /// Encode a text into a single L2-normalized embedding vector.
    /// Returns a zero vector for empty input rather than failing — callers
    /// typically treat zero-vectors as "no signal" and fall back to BM25.
    pub fn encode(&self, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Ok(vec![0.0; self.dim]);
        }
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow!("tokenize: {e}"))?;
        let ids: &[u32] = encoding.get_ids();

        let mut out = vec![0.0f32; self.dim];
        let mut count = 0usize;
        for &id in ids {
            let id = id as usize;
            if id >= self.vocab_size {
                continue;
            }
            let row = &self.embeddings[id * self.dim..(id + 1) * self.dim];
            for (o, v) in out.iter_mut().zip(row.iter()) {
                *o += *v;
            }
            count += 1;
        }
        if count == 0 {
            return Ok(out);
        }
        let inv = 1.0 / count as f32;
        for o in out.iter_mut() {
            *o *= inv;
        }
        l2_normalize(&mut out);
        Ok(out)
    }

    /// Bulk encode for indexing pipelines. Falls back to `encode` per item;
    /// kept as its own entry point so a SIMD/parallel implementation can
    /// slot in later without an API break.
    pub fn encode_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.encode(t)).collect()
    }
}

/// Cosine similarity between two L2-normalized vectors. Equivalent to a
/// dot product when both inputs are unit-length; we keep a divisor in
/// case a caller passes non-normalized vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "vector dim mismatch");
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = (na.sqrt() * nb.sqrt()).max(1e-12);
    dot / denom
}

fn l2_normalize(v: &mut [f32]) {
    let mut n = 0.0f32;
    for x in v.iter() {
        n += x * x;
    }
    let inv = 1.0 / n.sqrt().max(1e-12);
    for x in v.iter_mut() {
        *x *= inv;
    }
}

/// Where pluck caches embedding models. `$PLUCK_HOME/models/<id-safe>/`
/// or `~/.pluck/models/<id-safe>/`.
pub fn model_cache_dir(model_id: &str) -> Result<PathBuf> {
    let safe = model_id.replace('/', "__");
    Ok(pluck_root()?.join("models").join(safe))
}

/// Download `tokenizer.json` + `model.safetensors` from Hugging Face into
/// the cache dir if they're not already there. Returns the cache dir.
pub fn ensure_model_dir(model_id: &str) -> Result<PathBuf> {
    let dir = model_cache_dir(model_id)?;
    std::fs::create_dir_all(&dir).with_context(|| format!("create {dir:?}"))?;

    let tok = dir.join("tokenizer.json");
    let st = dir.join("model.safetensors");
    if tok.is_file() && st.is_file() {
        return Ok(dir);
    }

    tracing::info!(model = model_id, target = ?dir, "fetching embedding model");

    let api = hf_hub::api::sync::ApiBuilder::new()
        .with_cache_dir(dir.clone())
        .build()
        .context("hf-hub api build")?;
    let repo = api.model(model_id.to_string());

    let tok_src = repo
        .get("tokenizer.json")
        .context("download tokenizer.json")?;
    if tok_src != tok {
        std::fs::copy(&tok_src, &tok).context("copy tokenizer")?;
    }

    let st_src = repo
        .get("model.safetensors")
        .context("download model.safetensors")?;
    if st_src != st {
        std::fs::copy(&st_src, &st).context("copy safetensors")?;
    }

    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_of_identical_is_one() {
        let a = vec![0.1f32, 0.5, -0.3, 0.2];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_of_orthogonal_is_zero() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-5);
    }

    #[test]
    fn cosine_of_opposite_is_minus_one() {
        let a = vec![0.6f32, 0.8];
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn l2_normalize_makes_unit_vector() {
        let mut v = vec![3.0f32, 4.0]; // pre-normalize length = 5
        l2_normalize(&mut v);
        let len: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((len - 1.0).abs() < 1e-5);
    }

    #[test]
    fn selected_model_id_uses_env_override() {
        std::env::set_var("PLUCK_EMBED_MODEL", "minishlab/potion-retrieval-32M");
        assert_eq!(selected_model_id(), "minishlab/potion-retrieval-32M");
        std::env::remove_var("PLUCK_EMBED_MODEL");
        assert_eq!(selected_model_id(), DEFAULT_MODEL_ID);
    }

    /// End-to-end against the real model. Network-dependent and slow on
    /// the first run (downloads ~60 MB); cached afterwards. Gated by an
    /// env var so CI doesn't hit Hugging Face on every push.
    #[test]
    fn end_to_end_real_model_if_opted_in() {
        if std::env::var("PLUCK_RUN_MODEL_TESTS").is_err() {
            return;
        }
        let enc = StaticEncoder::load_or_fetch(&selected_model_id()).expect("load model");
        assert!(enc.dim() > 0);
        assert!(enc.vocab_size() > 0);

        let v_login = enc.encode("user login authentication").unwrap();
        let v_auth = enc.encode("auth flow handler").unwrap();
        let v_color = enc.encode("color palette accessibility").unwrap();

        assert_eq!(v_login.len(), enc.dim());

        // Auth-themed queries should be closer to each other than to a
        // tangential one. Static models are weaker than transformers but
        // this comparison is robust.
        let sim_close = cosine_similarity(&v_login, &v_auth);
        let sim_far = cosine_similarity(&v_login, &v_color);
        assert!(
            sim_close > sim_far,
            "expected auth/login > color: close={sim_close} far={sim_far}"
        );
    }
}
