//! The BERT encoder the local embedding model runs on (ADR-47), for inference on the CPU. Same weights, layout and math
//! as the reference implementation (candle-transformers' `models::bert`, HF `modeling_bert`), with two differences
//! measured on this machine (bge-small, 512 tokens, 10 cores):
//!
//! * the fused softmax over the last dimension: the reference's generic softmax took 101 ms per layer against 11 ms,
//!   so a whole-ticket query embedded in 1.6 s;
//! * no attention mask when no position is padding (every single text, so every query): the mask only adds zeros.
//!
//! `tests/local_model.rs` checks that its embeddings equal the reference's.

use candle_core::{Module, Result, Tensor};
use candle_nn::{embedding, layer_norm, linear, Embedding, LayerNorm, Linear, VarBuilder};
use candle_transformers::models::bert::{Config, HiddenAct};

struct Layer {
    query: Linear,
    key: Linear,
    value: Linear,
    attn_out: Linear,
    attn_norm: LayerNorm,
    intermediate: Linear,
    output: Linear,
    out_norm: LayerNorm,
}

pub struct Encoder {
    word: Embedding,
    position: Embedding,
    token_type: Embedding,
    norm: LayerNorm,
    layers: Vec<Layer>,
    heads: usize,
    head_dim: usize,
}

impl Encoder {
    /// Loads `embeddings.*` and `encoder.layer.N.*` (or the same under `<model_type>.`, as some checkpoints store
    /// them).
    pub fn load(vb: VarBuilder, c: &Config) -> Result<Self> {
        match Self::load_at(vb.clone(), c) {
            Ok(e) => Ok(e),
            Err(err) => match &c.model_type {
                Some(prefix) => Self::load_at(vb.pp(prefix), c).map_err(|_| err),
                None => Err(err),
            },
        }
    }

    fn load_at(vb: VarBuilder, c: &Config) -> Result<Self> {
        if !matches!(c.hidden_act, HiddenAct::Gelu) {
            candle_core::bail!("only the erf GELU activation is implemented (bge-small uses it)");
        }
        let h = c.hidden_size;
        let e = vb.pp("embeddings");
        let layers = (0..c.num_hidden_layers)
            .map(|i| {
                let l = vb.pp(format!("encoder.layer.{i}"));
                let a = l.pp("attention");
                Ok(Layer {
                    query: linear(h, h, a.pp("self.query"))?,
                    key: linear(h, h, a.pp("self.key"))?,
                    value: linear(h, h, a.pp("self.value"))?,
                    attn_out: linear(h, h, a.pp("output.dense"))?,
                    attn_norm: layer_norm(h, c.layer_norm_eps, a.pp("output.LayerNorm"))?,
                    intermediate: linear(h, c.intermediate_size, l.pp("intermediate.dense"))?,
                    output: linear(c.intermediate_size, h, l.pp("output.dense"))?,
                    out_norm: layer_norm(h, c.layer_norm_eps, l.pp("output.LayerNorm"))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            word: embedding(c.vocab_size, h, e.pp("word_embeddings"))?,
            position: embedding(c.max_position_embeddings, h, e.pp("position_embeddings"))?,
            token_type: embedding(c.type_vocab_size, h, e.pp("token_type_embeddings"))?,
            norm: layer_norm(h, c.layer_norm_eps, e.pp("LayerNorm"))?,
            layers,
            heads: c.num_attention_heads,
            head_dim: h / c.num_attention_heads,
        })
    }

    /// Hidden states `[batch, tokens, hidden]`. `mask` (`[batch, tokens]`, 1 = token, 0 = padding) is needed only
    /// when some position is padding.
    pub fn forward(&self, ids: &Tensor, types: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let (b, n) = ids.dims2()?;
        let positions = Tensor::arange(0u32, n as u32, ids.device())?;
        let x = (self.word.forward(ids)? + self.token_type.forward(types)?)?
            .broadcast_add(&self.position.forward(&positions)?)?;
        let mut x = self.norm.forward(&x)?;
        // Additive bias `[batch, 1, 1, tokens]`: 0 on tokens, f32::MIN on padding (as the reference).
        let bias = match mask {
            Some(m) => Some(
                ((m.to_dtype(x.dtype())?.ones_like()? - m.to_dtype(x.dtype())?)?
                    * f32::MIN as f64)?
                    .reshape((b, 1, 1, n))?,
            ),
            None => None,
        };
        let scale = (self.head_dim as f64).sqrt();
        for l in &self.layers {
            let split = |t: Tensor| -> Result<Tensor> {
                t.reshape((b, n, self.heads, self.head_dim))?
                    .transpose(1, 2)?
                    .contiguous()
            };
            let q = split(l.query.forward(&x)?)?;
            let k = split(l.key.forward(&x)?)?;
            let v = split(l.value.forward(&x)?)?;
            let mut scores = (q.matmul(&k.t()?)? / scale)?;
            if let Some(bias) = &bias {
                scores = scores.broadcast_add(bias)?;
            }
            let probs = candle_nn::ops::softmax_last_dim(&scores)?;
            let ctx = probs.matmul(&v)?.transpose(1, 2)?.contiguous()?.reshape((
                b,
                n,
                self.heads * self.head_dim,
            ))?;
            let a = l.attn_norm.forward(&(l.attn_out.forward(&ctx)? + &x)?)?;
            let i = l.intermediate.forward(&a)?.gelu_erf()?;
            x = l.out_norm.forward(&(l.output.forward(&i)? + &a)?)?;
        }
        Ok(x)
    }
}
