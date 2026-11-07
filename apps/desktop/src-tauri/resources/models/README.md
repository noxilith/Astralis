# In-app AI model

The Brain loads a small sentence-embedding model from `all-MiniLM-L6-v2/` in this folder
(`config.json`, `tokenizer.json`, `model.safetensors`). Run `./run.sh fetch-model` from the
repo root to download it (~90 MB). Without it, Astralis uses its built-in lexical embedder.
