//! Local embedding model (decision 2b, ADR-47): `bge-small-en-v1.5` (384-d, CLS pooling, L2-normalised) run on the
//! CPU with candle (pure Rust, no ONNX Runtime). Model files live in `<root>/models/<id>/`, pinned to one upstream
//! revision and verified by sha256; [`install`] fetches them (`agenium models download`, the app's Memory page).

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::Config;
use sha2::{Digest, Sha256};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

use crate::bert::Encoder;
use crate::Embedder;

/// A downloadable model file.
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    pub name: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub repo: &'static str,
    pub revision: &'static str,
    pub dim: usize,
    /// Instruction prepended to retrieval queries (not to stored facts), as the model card recommends.
    pub query_prefix: &'static str,
    pub files: &'static [ModelFile],
}

impl ModelSpec {
    pub fn url(&self, file: &ModelFile) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, file.name
        )
    }

    pub fn dir(&self, models_root: &Path) -> PathBuf {
        models_root.join(self.id)
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

pub const BGE_SMALL_EN: ModelSpec = ModelSpec {
    id: "bge-small-en-v1.5",
    repo: "BAAI/bge-small-en-v1.5",
    revision: "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a",
    dim: 384,
    query_prefix: "Represent this sentence for searching relevant passages: ",
    files: &[
        ModelFile {
            name: "config.json",
            sha256: "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
            bytes: 743,
        },
        ModelFile {
            name: "tokenizer.json",
            sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
            bytes: 711_396,
        },
        ModelFile {
            name: "model.safetensors",
            sha256: "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
            bytes: 133_466_304,
        },
    ],
};

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Every file present with the expected size (cheap check used at startup).
pub fn installed(spec: &ModelSpec, models_root: &Path) -> bool {
    let dir = spec.dir(models_root);
    spec.files.iter().all(|f| {
        std::fs::metadata(dir.join(f.name))
            .map(|m| m.len() == f.bytes)
            .unwrap_or(false)
    })
}

/// Downloads the model's files into `<models_root>/<id>/` (resumable per file: a verified file is kept), checking size
/// and sha256 before a file is moved into place. `progress` hears each step.
pub async fn install(
    spec: &ModelSpec,
    models_root: &Path,
    http: &agenium_llm::http::Http,
    progress: &(dyn Fn(&str) + Send + Sync),
) -> Result<(), String> {
    use std::io::Write;
    let dir = spec.dir(models_root);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for f in spec.files {
        let dst = dir.join(f.name);
        if matches!(sha256_file(&dst), Ok(h) if h == f.sha256) {
            progress(&format!("{}: ok", f.name));
            continue;
        }
        progress(&format!("{}: downloading {} KB", f.name, f.bytes >> 10));
        let part = dir.join(format!("{}.partial", f.name));
        let fail = |m: String| {
            let _ = std::fs::remove_file(&part);
            Err(format!("{}: {m}", f.name))
        };
        let mut resp = match http
            .client()
            .get(spec.url(f))
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => r,
            Err(e) => return fail(e.to_string()),
        };
        let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut n = 0u64;
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    n += chunk.len() as u64;
                    if n > f.bytes {
                        drop(out);
                        return fail("larger than expected".into());
                    }
                    if let Err(e) = out.write_all(&chunk) {
                        drop(out);
                        return fail(e.to_string());
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    drop(out);
                    return fail(e.to_string());
                }
            }
        }
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        let h = sha256_file(&part).map_err(|e| e.to_string())?;
        if n != f.bytes || h != f.sha256 {
            return fail(format!("checksum mismatch (got {h}, {n} bytes)"));
        }
        std::fs::rename(&part, &dst).map_err(|e| e.to_string())?;
        progress(&format!("{}: verified", f.name));
    }
    Ok(())
}

/// Full check: sizes and sha256 of every file. Returns the names that fail.
pub fn verify(spec: &ModelSpec, models_root: &Path) -> Vec<String> {
    let dir = spec.dir(models_root);
    spec.files
        .iter()
        .filter(|f| !matches!(sha256_file(&dir.join(f.name)), Ok(h) if h == f.sha256))
        .map(|f| f.name.to_owned())
        .collect()
}

pub struct LocalEmbedder {
    spec: ModelSpec,
    model: Encoder,
    tokenizer: Tokenizer,
    device: Device,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl LocalEmbedder {
    pub fn load(spec: ModelSpec, models_root: &Path) -> Result<LocalEmbedder, String> {
        let dir = spec.dir(models_root);
        if !installed(&spec, models_root) {
            return Err(format!(
                "embedding model {} is not installed in {} (run `agenium models download`)",
                spec.id,
                dir.display()
            ));
        }
        let device = Device::Cpu;
        let config: Config =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).map_err(err)?)
                .map_err(err)?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err)?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 512,
                ..Default::default()
            }))
            .map_err(err)?;
        // SAFETY: the file is memory-mapped read-only; it is not modified while the model lives (verified install).
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &[dir.join("model.safetensors")],
                DType::F32,
                &device,
            )
            .map_err(err)?
        };
        let model = Encoder::load(vb, &config).map_err(err)?;
        Ok(LocalEmbedder {
            spec,
            model,
            tokenizer,
            device,
        })
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let enc = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(err)?;
        let ids: Vec<Vec<u32>> = enc.iter().map(|e| e.get_ids().to_vec()).collect();
        let types: Vec<Vec<u32>> = enc.iter().map(|e| e.get_type_ids().to_vec()).collect();
        let mask: Vec<Vec<u32>> = enc
            .iter()
            .map(|e| e.get_attention_mask().to_vec())
            .collect();
        let padded = mask.iter().flatten().any(|m| *m == 0);
        let t = |v: Vec<Vec<u32>>| Tensor::new(v, &self.device).map_err(err);
        let mask = if padded { Some(t(mask)?) } else { None };
        let hidden = self
            .model
            .forward(&t(ids)?, &t(types)?, mask.as_ref())
            .map_err(err)?;
        // CLS pooling, then L2 normalisation.
        let cls = hidden.i((.., 0)).map_err(err)?;
        let norm = cls
            .sqr()
            .and_then(|x| x.sum_keepdim(1))
            .and_then(|x| x.sqrt())
            .map_err(err)?;
        let out = cls.broadcast_div(&norm).map_err(err)?;
        out.to_vec2::<f32>().map_err(err)
    }
}

impl Embedder for LocalEmbedder {
    fn model_id(&self) -> &str {
        self.spec.id
    }
    fn dim(&self) -> usize {
        self.spec.dim
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(32) {
            out.extend(self.embed_batch(chunk)?);
        }
        Ok(out)
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, String> {
        let q = format!("{}{text}", self.spec.query_prefix);
        Ok(self.embed_batch(&[q])?.remove(0))
    }
}
