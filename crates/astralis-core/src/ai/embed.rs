//! Embedders turn text into unit-length vectors whose dot product measures similarity.

use crate::ai::text::{content_words, stem};
use crate::error::Result;
use crate::util::fnv1a;

pub trait Embedder: Send {
    /// Identifies the model; vectors from different models are never compared.
    fn model_id(&self) -> &str;
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Built-in fallback that needs no model file: a hashed bag of stemmed words, bigrams and
/// character trigrams. Good at "same topic, shared vocabulary", weaker at synonyms.
pub struct LexicalEmbedder {
    dim: usize,
}

pub const LEXICAL_MODEL_ID: &str = "lexical-v1";

impl Default for LexicalEmbedder {
    fn default() -> Self {
        Self { dim: 1024 }
    }
}

impl LexicalEmbedder {
    fn add(&self, v: &mut [f32], feature: &str, weight: f32) {
        let h = fnv1a(feature) as u64;
        let idx = (h % self.dim as u64) as usize;
        let sign = if (h >> 63) & 1 == 0 { 1.0 } else { -1.0 };
        v[idx] += sign * weight;
    }

    pub fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut counts: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        let toks: Vec<String> = content_words(text).iter().map(|w| stem(w)).collect();
        for (i, w) in toks.iter().enumerate() {
            *counts.entry(format!("w:{w}")).or_default() += 1.0;
            if let Some(n) = toks.get(i + 1) {
                *counts.entry(format!("b:{w} {n}")).or_default() += 0.5;
            }
            let padded: Vec<char> = format!("^{w}$").chars().collect();
            for tri in padded.windows(3) {
                *counts
                    .entry(format!("c:{}", tri.iter().collect::<String>()))
                    .or_default() += 0.15;
            }
        }
        let mut v = vec![0.0f32; self.dim];
        for (feature, c) in counts {
            self.add(&mut v, &feature, 1.0 + c.ln_1p());
        }
        normalize(&mut v);
        v
    }
}

impl Embedder for LexicalEmbedder {
    fn model_id(&self) -> &str {
        LEXICAL_MODEL_ID
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}
