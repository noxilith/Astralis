//! A local language model that edits and writes notes: Qwen2.5 1.5B Instruct (Apache-2.0) as a
//! quantized GGUF, run on the CPU with llama.cpp. It powers "Ask Astralis" — rewriting,
//! expanding and tidying notes — entirely on this device. (Small on purpose: editing text
//! doesn't need a big model, and this one answers about 2.5x faster than Phi-3 mini did.)

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

use crate::ai::assistant::{ChatRole, Prompt};
use crate::error::{Error, Result};

/// The model file (in its folder). Its tokenizer is inside it.
pub const MODEL_FILE: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
/// Where to get it (the Qwen team's official release on Hugging Face, 1.1 GB).
pub const DOWNLOADS: &[(&str, &str)] =
    &[(MODEL_FILE, "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf")];
/// The model's context window (prompt + answer, in tokens).
pub const CONTEXT: usize = 4096;
/// How many prompt tokens are read per step.
const BATCH: usize = 512;

fn err(e: impl std::fmt::Display) -> Error {
    Error::Ai(format!("{e}"))
}

pub fn files_present(dir: &Path) -> bool {
    DOWNLOADS.iter().all(|(f, _)| dir.join(f).metadata().is_ok_and(|m| m.len() > 0))
}

/// llama.cpp is set up once per process.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<std::result::Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            LlamaBackend::init()
                .map(|mut b| {
                    b.void_logs(); // it's chatty on stderr
                    b
                })
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(err)
}

/// Threads for the math: every core (measured faster than only the fast cores on a hybrid
/// laptop CPU). `ASTRALIS_LLM_THREADS` overrides it.
fn threads() -> i32 {
    if let Some(n) = std::env::var("ASTRALIS_LLM_THREADS").ok().and_then(|v| v.parse().ok()) {
        return n;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16) as i32
}

/// Whether to run on the graphics chip. That needs a build with the `gpu-vulkan` or `gpu-metal`
/// feature (`ASTRALIS_GPU=1 ./run.sh app`), and is still off until it's been tested on more
/// hardware: on one laptop a GPU run coincided with a full system freeze. `ASTRALIS_LLM_GPU=1`
/// turns it on.
pub fn use_gpu() -> bool {
    std::env::var("ASTRALIS_LLM_GPU").is_ok_and(|v| v == "1")
}

pub struct Llm {
    /// The working memory (what the model has read so far), kept between replies so the shared
    /// start of the next prompt (persona, note, earlier conversation) isn't read again.
    /// Declared before `model`: it must be dropped first.
    ctx: LlamaContext<'static>,
    /// The tokens `ctx` holds, in order.
    seen: Vec<LlamaToken>,
    model: LlamaModel,
    /// Tokens that end a reply besides the model's own end-of-turn markers.
    stop: Vec<LlamaToken>,
    pub dir: PathBuf,
}

impl Llm {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(MODEL_FILE);
        if !path.exists() {
            return Err(err(format!("can't find the model ({})", path.display())));
        }
        let gpu = use_gpu();
        let params = LlamaModelParams::default().with_n_gpu_layers(if gpu { 999 } else { 0 });
        let model = LlamaModel::load_from_file(backend()?, &path, &params).map_err(err)?;
        let stop = ["<|im_end|>", "<|endoftext|>", "<|im_start|>"]
            .iter()
            .filter_map(|t| match model.vocab().tokenize(t.as_bytes(), false, true).as_slice() {
                [one] => Some(*one),
                _ => None,
            })
            .collect();
        let n = threads();
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(CONTEXT as u32))
            .with_n_batch(BATCH as u32)
            .with_n_threads(n)
            .with_n_threads_batch(n)
            // CPU only means nothing on the graphics chip: not its memory, not even single steps.
            .with_offload_kqv(gpu)
            .with_op_offload(gpu);
        let ctx = model.new_context(backend()?, params).map_err(err)?;
        // SAFETY: the context points at the C model, which lives on the heap (moving `model`
        // doesn't move it) and is freed after the context: `ctx` is declared first, so it's dropped first.
        let ctx = unsafe { std::mem::transmute::<LlamaContext<'_>, LlamaContext<'static>>(ctx) };
        Ok(Self { ctx, seen: Vec::new(), model, stop, dir: dir.to_path_buf() })
    }

    /// Tokens in a piece of text.
    pub fn count_tokens(&self, text: &str) -> usize {
        self.model.vocab().tokenize(text.as_bytes(), false, false).len()
    }

    /// The model's chat format (ChatML), with the system message as its own turn.
    fn format(prompt: &Prompt) -> String {
        let mut out = format!("<|im_start|>system\n{}<|im_end|>\n", prompt.system);
        for turn in &prompt.turns {
            let role = match turn.role {
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
            };
            out.push_str(&format!("<|im_start|>{role}\n{}<|im_end|>\n", turn.text));
        }
        out.push_str("<|im_start|>assistant\n");
        out
    }

    /// Forgets what's been read (after an error, when its state is unknown).
    fn reset(&mut self) {
        self.seen.clear();
        self.ctx.clear_kv_cache();
    }

    /// Reads one batch of tokens into the working memory; `last_logits` asks for the next-token
    /// guess after the last one.
    fn read(&mut self, batch: &mut LlamaBatch, tokens: &[LlamaToken], last_logits: bool) -> Result<()> {
        batch.clear();
        for (i, t) in tokens.iter().enumerate() {
            let pos = self.seen.len() as i32;
            batch.add(*t, pos, &[0], last_logits && i == tokens.len() - 1).map_err(err)?;
            self.seen.push(*t);
        }
        if let Err(e) = self.ctx.decode(batch) {
            self.reset();
            return Err(err(e));
        }
        Ok(())
    }

    /// Writes a reply, calling `on_text` with each new piece; return `false` from it to stop.
    pub fn generate(&mut self, prompt: &Prompt, max_new: usize, mut on_text: impl FnMut(&str) -> bool) -> Result<String> {
        let tokens = self.model.vocab().tokenize(Self::format(prompt).as_bytes(), false, true);
        let max_new = max_new.min(CONTEXT.saturating_sub(tokens.len() + 8));
        if max_new < 32 {
            return Err(Error::invalid("the note and conversation are too long for the model; start a new chat or shorten the note"));
        }

        // Keep what's already read up to where this prompt differs (at least the last token is
        // read again, so there's a fresh guess to answer from).
        let keep = self.seen.iter().zip(&tokens).take_while(|(a, b)| a == b).count().min(tokens.len() - 1);
        if self.ctx.clear_kv_cache_seq(Some(0), Some(keep as u32), None).is_err() {
            self.reset();
        }
        self.seen.truncate(keep);
        let keep = self.seen.len();

        // Read the rest of the prompt.
        let mut batch = LlamaBatch::new(BATCH, 1);
        let rest = &tokens[keep..];
        let chunks = rest.len().div_ceil(BATCH);
        for (i, chunk) in rest.chunks(BATCH).enumerate() {
            self.read(&mut batch, chunk, i == chunks - 1)?;
        }

        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7);
        let mut sampler = LlamaSampler::chain_simple([
            // Light on repeats: editing a note means saying much of it again.
            LlamaSampler::penalties(self.model.n_vocab(), 64, 1.05, 0.0, 0.0),
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(0.4),
            LlamaSampler::dist(seed),
        ]);

        // Write the reply, a token at a time.
        let mut reply = String::new();
        let mut pending: Vec<u8> = Vec::new(); // bytes of a character split across tokens
        for _ in 0..max_new {
            let next = sampler.sample(&self.ctx, batch.n_tokens() - 1);
            let vocab = self.model.vocab();
            if vocab.is_eog(next) || self.stop.contains(&next) {
                break;
            }
            pending.extend(vocab.token_to_piece(next, false, None));
            let valid = match std::str::from_utf8(&pending) {
                Ok(s) => s.len(),
                Err(e) => e.valid_up_to(),
            };
            if valid > 0 {
                let piece = String::from_utf8_lossy(&pending[..valid]).into_owned();
                pending.drain(..valid);
                reply.push_str(&piece);
                if !on_text(&piece) {
                    break;
                }
            }
            self.read(&mut batch, &[next], true)?;
        }
        Ok(reply.trim().to_string())
    }
}
