//! The Brain: Astralis's in-app AI memory. It embeds notes into vectors (stored next to the
//! notes, never leaving the device) and builds every AI feature on top of them:
//! related notes, duplicate detection, smart organization (clustering), tag suggestions and
//! extractive summaries.
//!
//! The model is loaded only when needed and dropped again after `idle_timeout`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ai::embed::{Embedder, LEXICAL_MODEL_ID, LexicalEmbedder, dot, normalize};
use crate::ai::text::{Corpus, key_terms, sentences, stem, strip_markdown, words};
use crate::error::{Error, Result};
use crate::links::parse_wikilinks;
use crate::notes::{NoteKind, NoteRef, normalize_tag};
use crate::store::Store;
use crate::util::{Timestamp, fnv1a, uuid_col};

const MAX_EMBED_CHARS: usize = 4000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainStatus {
    pub loaded: bool,
    /// The model that is (or would be) used.
    pub model: String,
    /// True when the neural model files are present and support was compiled in.
    pub neural: bool,
    pub indexed: i64,
    pub pending: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedNote {
    pub note: NoteRef,
    pub score: f32,
    /// Already linked in either direction (so not a *new* connection).
    pub linked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePair {
    pub a: NoteRef,
    pub b: NoteRef,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub label: String,
    pub suggested_tag: String,
    pub notes: Vec<NoteRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagSuggestion {
    pub tag: String,
    pub score: f32,
    /// "keyword" (from the note's own text) or "similar" (used on similar notes).
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub sentences: Vec<String>,
    pub text: String,
}

struct Indexed {
    id: Uuid,
    title: String,
    text: String,
    vector: Vec<f32>,
}

pub struct Brain {
    model_dir: Option<PathBuf>,
    embedder: Option<Box<dyn Embedder>>,
    last_used: Instant,
    pub idle_timeout: Duration,
}

fn embedding_text(title: &str, body: &str) -> String {
    let mut t = format!("{}\n{}", title.trim(), strip_markdown(body));
    if t.len() > MAX_EMBED_CHARS {
        let mut cut = MAX_EMBED_CHARS;
        while !t.is_char_boundary(cut) {
            cut -= 1;
        }
        t.truncate(cut);
    }
    t
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

impl Brain {
    /// `model_dir` holds the neural model; without it (or without the `ml` feature) the Brain
    /// uses the built-in lexical embedder.
    pub fn new(model_dir: Option<PathBuf>) -> Self {
        Self {
            model_dir,
            embedder: None,
            last_used: Instant::now(),
            idle_timeout: Duration::from_secs(120),
        }
    }

    pub fn neural_available(&self) -> bool {
        #[cfg(feature = "ml")]
        {
            self.model_dir
                .as_deref()
                .is_some_and(crate::ai::bert::model_files_present)
        }
        #[cfg(not(feature = "ml"))]
        {
            false
        }
    }

    /// The model id that will be used, without loading anything.
    pub fn model_id(&self) -> String {
        match &self.embedder {
            Some(e) => e.model_id().to_owned(),
            None if self.neural_available() => {
                let name = self
                    .model_dir
                    .as_ref()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                format!("bert:{name}")
            }
            None => LEXICAL_MODEL_ID.to_owned(),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.embedder.is_some()
    }

    fn embedder(&mut self) -> Result<&mut Box<dyn Embedder>> {
        self.last_used = Instant::now();
        if self.embedder.is_none() {
            let loaded: Box<dyn Embedder> = self
                .load_neural()
                .unwrap_or_else(|| Box::new(LexicalEmbedder::default()));
            self.embedder = Some(loaded);
        }
        Ok(self.embedder.as_mut().expect("just loaded"))
    }

    #[cfg(feature = "ml")]
    fn load_neural(&self) -> Option<Box<dyn Embedder>> {
        let dir = self
            .model_dir
            .as_deref()
            .filter(|d| crate::ai::bert::model_files_present(d))?;
        match crate::ai::bert::BertEmbedder::load(dir) {
            Ok(m) => Some(Box::new(m)),
            Err(e) => {
                eprintln!("astralis: neural model failed to load, using lexical fallback: {e}");
                None
            }
        }
    }

    #[cfg(not(feature = "ml"))]
    fn load_neural(&self) -> Option<Box<dyn Embedder>> {
        None
    }

    /// Frees the model if it hasn't been used for `idle_timeout`. Returns true if it unloaded.
    pub fn unload_if_idle(&mut self) -> bool {
        if self.embedder.is_some() && self.last_used.elapsed() >= self.idle_timeout {
            self.embedder = None;
            return true;
        }
        false
    }

    pub fn unload(&mut self) {
        self.embedder = None;
    }

    pub fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embedder()?.embed(texts)
    }

    pub fn status(&self, store: &Store) -> Result<BrainStatus> {
        let model = self.model_id();
        let indexed: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM embeddings WHERE model = ?1",
            [&model],
            |r| r.get(0),
        )?;
        let pending = store.pending_embedding_count(&model)?;
        Ok(BrainStatus {
            loaded: self.is_loaded(),
            model,
            neural: self.neural_available(),
            indexed,
            pending,
        })
    }

    /// Embeds up to `limit` new or changed notes. Returns how many were processed.
    /// Only call this when there's work (`pending > 0`) to avoid loading the model needlessly.
    pub fn index(&mut self, store: &Store, limit: usize) -> Result<usize> {
        let model = self.model_id();
        let todo = store.notes_needing_embedding(&model, limit)?;
        if todo.is_empty() {
            return Ok(0);
        }
        let texts: Vec<String> = todo.iter().map(|t| t.text.clone()).collect();
        let vectors = self.embed(&texts)?;
        let model = self.model_id(); // may change if the neural model failed to load
        for (job, v) in todo.iter().zip(vectors) {
            store.save_embedding(job.id, &model, fnv1a(&job.text), job.updated_at, &v)?;
        }
        Ok(todo.len())
    }

    /// Indexes everything that's pending.
    pub fn index_all(&mut self, store: &Store) -> Result<usize> {
        let mut total = 0;
        loop {
            let n = self.index(store, 64)?;
            total += n;
            if n == 0 {
                return Ok(total);
            }
        }
    }

    /// Indexes pending notes, then loads one vault's (plaintext, active) note vectors.
    fn corpus(&mut self, store: &Store, vault: Option<Uuid>) -> Result<Vec<Indexed>> {
        let vault = match vault {
            Some(v) => v,
            None => store.default_vault()?,
        };
        self.index_all(store)?;
        store.load_index(&self.model_id(), vault)
    }

    /// Notes most similar in meaning to `note_id`, within its vault.
    pub fn related(&mut self, store: &Store, note_id: Uuid, k: usize) -> Result<Vec<RelatedNote>> {
        let note = store.get_note(note_id)?;
        if note.encrypted || note.kind != NoteKind::Note {
            return Ok(vec![]);
        }
        let all = self.corpus(store, Some(note.vault_id))?;
        let Some(me) = all.iter().find(|n| n.id == note_id) else {
            return Ok(vec![]);
        };
        let linked = store.linked_ids(note_id, note.vault_id)?;
        let min = if self.model_id() == LEXICAL_MODEL_ID {
            0.12
        } else {
            0.35
        };
        let mut scored: Vec<RelatedNote> = all
            .iter()
            .filter(|n| n.id != note_id)
            .map(|n| RelatedNote {
                note: NoteRef {
                    id: n.id,
                    title: n.title.clone(),
                },
                score: dot(&me.vector, &n.vector),
                linked: linked.contains(&n.id),
            })
            .filter(|r| r.score >= min)
            .collect();
        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored.truncate(k);
        Ok(scored)
    }

    /// Default similarity above which two notes are reported as likely duplicates.
    pub fn default_duplicate_threshold(&self) -> f32 {
        if self.model_id() == LEXICAL_MODEL_ID {
            0.8
        } else {
            0.92
        }
    }

    /// Pairs of notes that are (nearly) the same, most similar first.
    pub fn duplicates(
        &mut self,
        store: &Store,
        vault: Option<Uuid>,
        threshold: Option<f32>,
    ) -> Result<Vec<DuplicatePair>> {
        let threshold = threshold.unwrap_or_else(|| self.default_duplicate_threshold());
        let all = self.corpus(store, vault)?;
        let mut pairs = Vec::new();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                let s = dot(&all[i].vector, &all[j].vector);
                if s >= threshold {
                    pairs.push(DuplicatePair {
                        a: NoteRef {
                            id: all[i].id,
                            title: all[i].title.clone(),
                        },
                        b: NoteRef {
                            id: all[j].id,
                            title: all[j].title.clone(),
                        },
                        score: s,
                    });
                }
            }
        }
        pairs.sort_by(|a, b| b.score.total_cmp(&a.score));
        pairs.truncate(200);
        Ok(pairs)
    }

    /// Groups notes by topic (spherical k-means) and names each group by its key terms.
    pub fn organize(&mut self, store: &Store, vault: Option<Uuid>) -> Result<Vec<Cluster>> {
        let all = self.corpus(store, vault)?;
        if all.len() < 4 {
            return Ok(vec![]);
        }
        let k = ((all.len() as f32 / 2.0).sqrt().round() as usize)
            .clamp(2, 12)
            .min(all.len());
        let vectors: Vec<&[f32]> = all.iter().map(|n| n.vector.as_slice()).collect();
        let assignment = kmeans(&vectors, k, 40);

        let tokens: Vec<Vec<String>> = all
            .iter()
            .map(|n| words(&n.text).iter().map(|w| stem(w)).collect())
            .collect();
        let corpus = Corpus::new(tokens.iter().map(|t| t.as_slice()));

        let mut groups: Vec<Vec<usize>> = vec![Vec::new(); k];
        for (i, c) in assignment.iter().enumerate() {
            groups[*c].push(i);
        }
        let mut clusters = Vec::new();
        let mut unsorted = Vec::new();
        for members in groups.into_iter().filter(|g| !g.is_empty()) {
            if members.len() == 1 {
                unsorted.extend(members);
                continue;
            }
            let text: String = members
                .iter()
                .map(|&i| all[i].text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let terms = key_terms(&text, &corpus, 3);
            let label = if terms.is_empty() {
                "Misc".to_owned()
            } else {
                terms
                    .iter()
                    .map(|(t, _)| t.as_str())
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            let suggested_tag = terms
                .first()
                .and_then(|(t, _)| normalize_tag(t))
                .unwrap_or_default();
            clusters.push(Cluster {
                label,
                suggested_tag,
                notes: members
                    .iter()
                    .map(|&i| NoteRef {
                        id: all[i].id,
                        title: all[i].title.clone(),
                    })
                    .collect(),
            });
        }
        clusters.sort_by(|a, b| b.notes.len().cmp(&a.notes.len()));
        if !unsorted.is_empty() {
            clusters.push(Cluster {
                label: "Unsorted".into(),
                suggested_tag: String::new(),
                notes: unsorted
                    .iter()
                    .map(|&i| NoteRef {
                        id: all[i].id,
                        title: all[i].title.clone(),
                    })
                    .collect(),
            });
        }
        Ok(clusters)
    }

    /// Suggests tags for a note: tags used on similar notes, plus key phrases from the note
    /// ranked by how well they capture its meaning. Existing tags are excluded.
    pub fn suggest_tags(
        &mut self,
        store: &Store,
        note_id: Uuid,
        max: usize,
    ) -> Result<Vec<TagSuggestion>> {
        let note = store.get_note(note_id)?;
        if note.encrypted || note.kind != NoteKind::Note {
            return Ok(vec![]);
        }
        let all = self.corpus(store, Some(note.vault_id))?;
        let Some(me) = all.iter().find(|n| n.id == note_id) else {
            return Ok(vec![]);
        };
        let existing: HashSet<String> = note.tags.iter().cloned().collect();
        let mut scores: HashMap<String, (f32, &'static str)> = HashMap::new();

        // 1. Tags from the most similar notes, weighted by similarity.
        let tags_by_note = store.tags_by_note()?;
        let mut neighbours: Vec<(f32, Uuid)> = all
            .iter()
            .filter(|n| n.id != note_id)
            .map(|n| (dot(&me.vector, &n.vector), n.id))
            .collect();
        neighbours.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (sim, id) in neighbours.into_iter().take(8).filter(|(s, _)| *s > 0.25) {
            for tag in tags_by_note.get(&id).into_iter().flatten() {
                let e = scores.entry(tag.clone()).or_insert((0.0, "similar"));
                e.0 += sim * 0.6;
            }
        }

        // 2. Key phrases, re-ranked by semantic closeness to the whole note.
        let tokens: Vec<Vec<String>> = all
            .iter()
            .map(|n| words(&n.text).iter().map(|w| stem(w)).collect())
            .collect();
        let corpus = Corpus::new(tokens.iter().map(|t| t.as_slice()));
        let candidates = key_terms(&me.text, &corpus, 12);
        if !candidates.is_empty() {
            let max_tfidf = candidates[0].1.max(1e-6);
            let phrases: Vec<String> = candidates.iter().map(|(t, _)| t.clone()).collect();
            let vecs = self.embed(&phrases)?;
            for ((phrase, tfidf), v) in candidates.iter().zip(vecs) {
                let Some(tag) = normalize_tag(phrase) else {
                    continue;
                };
                let score = 0.5 * dot(&me.vector, &v).max(0.0) + 0.5 * (tfidf / max_tfidf);
                let e = scores.entry(tag).or_insert((0.0, "keyword"));
                e.0 = e.0.max(score);
            }
        }

        let mut out: Vec<TagSuggestion> = scores
            .into_iter()
            .filter(|(t, _)| !existing.contains(t))
            .map(|(tag, (score, source))| TagSuggestion {
                tag,
                score,
                source: source.into(),
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.tag.cmp(&b.tag)));
        out.truncate(max);
        Ok(out)
    }

    /// Extractive summary: picks the sentences that best represent the text (central to its
    /// meaning, not redundant with each other), in their original order.
    pub fn summarize_text(&mut self, text: &str, max_sentences: usize) -> Result<Summary> {
        let sents = sentences(&strip_markdown(text));
        let max_sentences = max_sentences.max(1);
        if sents.len() <= max_sentences {
            return Ok(Summary {
                text: sents.join(" "),
                sentences: sents,
            });
        }
        let vecs = self.embed(&sents)?;
        let mut centroid = vec![0.0f32; vecs[0].len()];
        for v in &vecs {
            centroid.iter_mut().zip(v).for_each(|(c, x)| *c += x);
        }
        normalize(&mut centroid);
        let n = sents.len() as f32;
        let relevance: Vec<f32> = vecs
            .iter()
            .enumerate()
            .map(|(i, v)| {
                dot(v, &centroid) + 0.08 * (1.0 - i as f32 / n) + if i == 0 { 0.05 } else { 0.0 }
            })
            .collect();

        // Maximal marginal relevance: balance relevance against redundancy.
        let lambda = 0.7;
        let mut chosen: Vec<usize> = Vec::new();
        while chosen.len() < max_sentences {
            let best = (0..sents.len())
                .filter(|i| !chosen.contains(i))
                .max_by(|&a, &b| {
                    let mmr = |i: usize| {
                        let redundancy = chosen
                            .iter()
                            .map(|&j| dot(&vecs[i], &vecs[j]))
                            .fold(0.0f32, f32::max);
                        lambda * relevance[i] - (1.0 - lambda) * redundancy
                    };
                    mmr(a).total_cmp(&mmr(b))
                });
            match best {
                Some(i) => chosen.push(i),
                None => break,
            }
        }
        chosen.sort_unstable();
        let picked: Vec<String> = chosen.into_iter().map(|i| sents[i].clone()).collect();
        Ok(Summary {
            text: picked.join(" "),
            sentences: picked,
        })
    }

    pub fn summarize_note(
        &mut self,
        store: &Store,
        note_id: Uuid,
        max_sentences: usize,
    ) -> Result<Summary> {
        let note = store.get_note(note_id)?;
        if note.kind != NoteKind::Note {
            return Err(Error::invalid("only text notes can be summarized"));
        }
        if note.body.trim().is_empty() {
            return Err(Error::invalid("note is empty"));
        }
        self.summarize_text(&note.body, max_sentences)
    }
}

/// Spherical k-means with deterministic k-means++ seeding. Returns a cluster index per vector.
fn kmeans(vectors: &[&[f32]], k: usize, iterations: usize) -> Vec<usize> {
    let n = vectors.len();
    let dim = vectors[0].len();
    let mut rng: u64 = 0x9e3779b97f4a7c15;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut centroids: Vec<Vec<f32>> = vec![vectors[0].to_vec()];
    while centroids.len() < k {
        let d: Vec<f64> = vectors
            .iter()
            .map(|v| {
                let best = centroids.iter().map(|c| dot(v, c)).fold(f32::MIN, f32::max);
                ((1.0 - best).max(0.0) as f64).powi(2)
            })
            .collect();
        let total: f64 = d.iter().sum();
        if total <= 0.0 {
            break;
        }
        let mut target = next() * total;
        let mut pick = n - 1;
        for (i, w) in d.iter().enumerate() {
            target -= w;
            if target <= 0.0 {
                pick = i;
                break;
            }
        }
        centroids.push(vectors[pick].to_vec());
    }
    let k = centroids.len();
    let mut assign = vec![0usize; n];
    for _ in 0..iterations {
        let mut changed = false;
        for (i, v) in vectors.iter().enumerate() {
            let best = (0..k)
                .max_by(|&a, &b| dot(v, &centroids[a]).total_cmp(&dot(v, &centroids[b])))
                .unwrap_or(0);
            if assign[i] != best {
                assign[i] = best;
                changed = true;
            }
        }
        let mut sums = vec![vec![0.0f32; dim]; k];
        for (i, v) in vectors.iter().enumerate() {
            sums[assign[i]]
                .iter_mut()
                .zip(v.iter())
                .for_each(|(s, x)| *s += x);
        }
        for (c, mut s) in sums.into_iter().enumerate() {
            if s.iter().any(|x| *x != 0.0) {
                normalize(&mut s);
                centroids[c] = s;
            }
        }
        if !changed {
            break;
        }
    }
    assign
}

pub(crate) struct EmbedJob {
    pub id: Uuid,
    pub text: String,
    pub updated_at: Timestamp,
}

impl Store {
    fn pending_embedding_count(&self, model: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM notes n LEFT JOIN embeddings e ON e.note_id = n.id
             WHERE n.cipher IS NULL AND n.kind = 'note' AND (e.note_id IS NULL OR e.model != ?1 OR e.note_updated_at != n.updated_at)",
            [model],
            |r| r.get(0),
        )?)
    }

    pub(crate) fn notes_needing_embedding(
        &self,
        model: &str,
        limit: usize,
    ) -> Result<Vec<EmbedJob>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT n.id, n.title, n.body, n.updated_at, e.model, e.content_hash
             FROM notes n LEFT JOIN embeddings e ON e.note_id = n.id
             WHERE n.cipher IS NULL AND n.kind = 'note' AND (e.note_id IS NULL OR e.model != ?1 OR e.note_updated_at != n.updated_at)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![model, limit as i64], |r| {
            Ok((
                uuid_col(r, 0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })?;
        let mut jobs = Vec::new();
        for row in rows {
            let (id, title, body, updated_at, old_model, old_hash) = row?;
            let text = embedding_text(&title, &body);
            // Metadata-only change (e.g. pin/archive): content identical → just bump the stamp.
            if old_model.as_deref() == Some(model) && old_hash == Some(fnv1a(&text)) {
                self.conn.execute(
                    "UPDATE embeddings SET note_updated_at = ?2 WHERE note_id = ?1",
                    params![id.as_bytes(), updated_at],
                )?;
                continue;
            }
            jobs.push(EmbedJob {
                id,
                text,
                updated_at,
            });
        }
        Ok(jobs)
    }

    fn save_embedding(
        &self,
        id: Uuid,
        model: &str,
        hash: i64,
        updated_at: Timestamp,
        v: &[f32],
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO embeddings(note_id, model, content_hash, note_updated_at, vector) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(note_id) DO UPDATE SET model = excluded.model, content_hash = excluded.content_hash,
                note_updated_at = excluded.note_updated_at, vector = excluded.vector",
            params![id.as_bytes(), model, hash, updated_at, to_blob(v)],
        )?;
        Ok(())
    }

    fn load_index(&self, model: &str, vault: Uuid) -> Result<Vec<Indexed>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.title, n.body, e.vector FROM notes n JOIN embeddings e ON e.note_id = n.id
             WHERE n.cipher IS NULL AND n.kind = 'note' AND n.archived_at IS NULL
               AND e.model = ?1 AND n.vault_id = ?2",
        )?;
        let rows = stmt.query_map(params![model, vault.as_bytes()], |r| {
            let title: String = r.get(1)?;
            let body: String = r.get(2)?;
            let blob: Vec<u8> = r.get(3)?;
            Ok(Indexed {
                id: uuid_col(r, 0)?,
                text: embedding_text(&title, &body),
                title,
                vector: from_blob(&blob),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn tags_by_note(&self) -> Result<HashMap<Uuid, Vec<String>>> {
        let mut stmt = self.conn.prepare(
            "SELECT nt.note_id, t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id",
        )?;
        let mut out: HashMap<Uuid, Vec<String>> = HashMap::new();
        for row in stmt.query_map([], |r| Ok((uuid_col(r, 0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            out.entry(id).or_default().push(name);
        }
        Ok(out)
    }

    /// Ids of notes linked to or from this note.
    fn linked_ids(&self, id: Uuid, vault: Uuid) -> Result<HashSet<Uuid>> {
        let mut out = HashSet::new();
        let note: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT title, body FROM notes WHERE id = ?1",
                [id.as_bytes()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((title, body)) = note else {
            return Ok(out);
        };
        for link in parse_wikilinks(&body) {
            out.extend(
                self.find_by_title(vault, &link.target)?
                    .into_iter()
                    .map(|n| n.id),
            );
        }
        if !title.trim().is_empty() {
            let mut stmt = self
                .conn
                .prepare_cached("SELECT source_id FROM links WHERE target_title = ?1")?;
            for row in stmt.query_map([title.trim().to_lowercase()], |r| uuid_col(r, 0))? {
                out.insert(row?);
            }
        }
        Ok(out)
    }
}
