//! Neural sentence embeddings with a small BERT-family model (e.g. all-MiniLM-L6-v2),
//! running in-process on the CPU via candle. The model directory must contain
//! `config.json`, `tokenizer.json` and `model.safetensors`.

use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

use crate::ai::embed::Embedder;
use crate::error::{Error, Result};

const MAX_TOKENS: usize = 256;
const BATCH: usize = 16;

pub struct BertEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    id: String,
}

fn ai_err(e: impl std::fmt::Display) -> Error {
    Error::Ai(e.to_string())
}

pub fn model_files_present(dir: &Path) -> bool {
    ["config.json", "tokenizer.json", "model.safetensors"]
        .iter()
        .all(|f| dir.join(f).is_file())
}

impl BertEmbedder {
    pub fn load(dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let config: Config =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(ai_err)?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(ai_err)?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        let weights = std::fs::read(dir.join("model.safetensors"))?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DTYPE, &device).map_err(ai_err)?;
        let model = BertModel::load(vb, &config).map_err(ai_err)?;
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Self {
            model,
            tokenizer,
            device,
            id: format!("bert:{name}"),
        })
    }

    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let enc = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(ai_err)?;
        let ids: Vec<Tensor> = enc
            .iter()
            .map(|e| Tensor::new(e.get_ids(), &self.device))
            .collect::<candle_core::Result<_>>()
            .map_err(ai_err)?;
        let mask: Vec<Tensor> = enc
            .iter()
            .map(|e| Tensor::new(e.get_attention_mask(), &self.device))
            .collect::<candle_core::Result<_>>()
            .map_err(ai_err)?;
        let ids = Tensor::stack(&ids, 0).map_err(ai_err)?;
        let mask = Tensor::stack(&mask, 0).map_err(ai_err)?;
        let type_ids = ids.zeros_like().map_err(ai_err)?;
        let out = self
            .model
            .forward(&ids, &type_ids, Some(&mask))
            .map_err(ai_err)?; // [b, t, h]

        // Mean pooling over real (non-padding) tokens, then L2 normalize.
        let m = mask
            .to_dtype(DType::F32)
            .map_err(ai_err)?
            .unsqueeze(2)
            .map_err(ai_err)?;
        let summed = out
            .broadcast_mul(&m)
            .and_then(|x| x.sum(1))
            .map_err(ai_err)?;
        let counts = m
            .sum(1)
            .and_then(|c| c.clamp(1e-9, f32::MAX))
            .map_err(ai_err)?;
        let pooled = summed.broadcast_div(&counts).map_err(ai_err)?;
        let norm = pooled
            .sqr()
            .and_then(|x| x.sum_keepdim(1))
            .and_then(|x| x.sqrt())
            .and_then(|x| x.clamp(1e-12, f32::MAX))
            .map_err(ai_err)?;
        let normalized = pooled.broadcast_div(&norm).map_err(ai_err)?;
        normalized.to_vec2::<f32>().map_err(ai_err)
    }
}

impl Embedder for BertEmbedder {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH) {
            out.extend(self.embed_batch(chunk)?);
        }
        Ok(out)
    }
}
