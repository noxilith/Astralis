//! `[[Wiki links]]` between notes, backlinks and the knowledge graph. Links resolve by title
//! within a vault; encrypted vaults have no link index (titles there are ciphertext).

use std::collections::{HashMap, HashSet};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;
use crate::notes::{NoteKind, NoteRef};
use crate::store::Store;
use crate::util::uuid_col;

/// A parsed `[[target|alias]]` occurrence. Offsets are byte offsets of the whole `[[...]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    pub target: String,
    pub start: usize,
    pub end: usize,
    /// Byte range of the target text inside the link.
    pub target_range: (usize, usize),
}

/// Finds `[[Target]]`, `[[Target|Alias]]` and `[[Target#Heading]]` links.
pub fn parse_wikilinks(text: &str) -> Vec<WikiLink> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(open) = text[i..].find("[[") {
        let start = i + open;
        let inner_start = start + 2;
        let Some(close) = text[inner_start..].find("]]") else {
            break;
        };
        let inner_end = inner_start + close;
        let inner = &text[inner_start..inner_end];
        if !inner.contains('\n') && !inner.contains("[[") {
            let target_len = inner.find(['|', '#']).unwrap_or(inner.len());
            let raw = &inner[..target_len];
            let target = raw.trim();
            if !target.is_empty() {
                let lead = raw.len() - raw.trim_start().len();
                let ts = inner_start + lead;
                out.push(WikiLink {
                    target: target.to_owned(),
                    start,
                    end: inner_end + 2,
                    target_range: (ts, ts + target.len()),
                });
            }
            i = inner_end + 2;
        } else {
            i = inner_start;
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backlink {
    pub note: NoteRef,
    /// The line of the linking note that contains the link.
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingLink {
    pub title: String,
    /// Empty when no note with that title exists yet (an "unresolved" link).
    pub notes: Vec<NoteRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: Uuid,
    pub kind: NoteKind,
    pub title: String,
    pub tags: Vec<String>,
    pub degree: usize,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub source: Uuid,
    pub target: Uuid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl Store {
    fn vault_or_default(&self, vault: Option<Uuid>) -> Result<Uuid> {
        match vault {
            Some(v) => Ok(v),
            None => self.default_vault(),
        }
    }

    pub(crate) fn sync_links(&self, id: Uuid, body: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM links WHERE source_id = ?1", [id.as_bytes()])?;
        let targets: HashSet<String> = parse_wikilinks(body)
            .into_iter()
            .map(|l| l.target.to_lowercase())
            .collect();
        let mut stmt = self.conn.prepare_cached(
            "INSERT OR IGNORE INTO links(source_id, target_title) VALUES (?1, ?2)",
        )?;
        for t in targets {
            stmt.execute(params![id.as_bytes(), t])?;
        }
        Ok(())
    }

    /// After a rename, points every `[[Old]]` link in the vault at `[[New]]` (keeping aliases
    /// and headings).
    pub(crate) fn rewrite_links(
        &self,
        vault: Uuid,
        old: &str,
        new: &str,
        except: Uuid,
    ) -> Result<()> {
        let old_lc = old.trim().to_lowercase();
        let new = new.trim();
        if new.is_empty() || old_lc == new.to_lowercase() {
            return Ok(());
        }
        let sources: Vec<(Uuid, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT n.id, n.body FROM links l JOIN notes n ON n.id = l.source_id
                 WHERE l.target_title = ?1 AND n.id != ?2 AND n.vault_id = ?3 AND n.cipher IS NULL",
            )?;
            stmt.query_map(params![old_lc, except.as_bytes(), vault.as_bytes()], |r| {
                Ok((uuid_col(r, 0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        for (id, body) in sources {
            let mut out = String::with_capacity(body.len());
            let mut last = 0;
            for link in parse_wikilinks(&body) {
                if link.target.to_lowercase() == old_lc {
                    out.push_str(&body[last..link.target_range.0]);
                    out.push_str(new);
                    last = link.target_range.1;
                }
            }
            out.push_str(&body[last..]);
            self.conn.execute(
                "UPDATE notes SET body = ?2 WHERE id = ?1",
                params![id.as_bytes(), out],
            )?;
            self.sync_links(id, &out)?;
        }
        Ok(())
    }

    /// Notes in the same vault that link to this note.
    pub fn backlinks(&self, id: Uuid) -> Result<Vec<Backlink>> {
        let note = self.get_note(id)?;
        if note.encrypted || note.title.trim().is_empty() {
            return Ok(vec![]);
        }
        let title_lc = note.title.trim().to_lowercase();
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.title, n.body FROM links l JOIN notes n ON n.id = l.source_id
             WHERE l.target_title = ?1 AND n.id != ?2 AND n.vault_id = ?3 AND n.archived_at IS NULL
             ORDER BY n.updated_at DESC",
        )?;
        let rows = stmt.query_map(
            params![title_lc, id.as_bytes(), note.vault_id.as_bytes()],
            |r| {
                Ok((
                    uuid_col(r, 0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (sid, stitle, sbody) = row?;
            let context = sbody
                .lines()
                .find(|line| {
                    parse_wikilinks(line)
                        .iter()
                        .any(|l| l.target.to_lowercase() == title_lc)
                })
                .unwrap_or("")
                .trim()
                .chars()
                .take(240)
                .collect();
            out.push(Backlink {
                note: NoteRef {
                    id: sid,
                    title: stitle,
                },
                context,
            });
        }
        Ok(out)
    }

    /// Links in this note, resolved to notes in its vault where possible.
    pub fn outgoing_links(&self, id: Uuid) -> Result<Vec<OutgoingLink>> {
        let note = self.get_note(id)?;
        if note.encrypted || note.kind != NoteKind::Note {
            return Ok(vec![]);
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for link in parse_wikilinks(&note.body) {
            if !seen.insert(link.target.to_lowercase()) {
                continue;
            }
            let notes = self.find_by_title(note.vault_id, &link.target)?;
            out.push(OutgoingLink {
                title: link.target,
                notes,
            });
        }
        Ok(out)
    }

    /// Exact (case-insensitive) title lookup within a plain vault.
    pub fn find_by_title(&self, vault: Uuid, title: &str) -> Result<Vec<NoteRef>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, title FROM notes WHERE lower(title) = lower(?1) AND vault_id = ?2 AND cipher IS NULL
             ORDER BY archived_at IS NOT NULL, updated_at DESC",
        )?;
        let rows = stmt.query_map(params![title.trim(), vault.as_bytes()], |r| {
            Ok(NoteRef {
                id: uuid_col(r, 0)?,
                title: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Title autocomplete for `[[` links: prefix matches first, then substring matches.
    pub fn suggest_titles(
        &self,
        vault: Option<Uuid>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<NoteRef>> {
        let vault = self.vault_or_default(vault)?;
        let q = query.trim().to_lowercase().replace(['%', '_'], "");
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, title FROM notes
             WHERE vault_id = ?3 AND cipher IS NULL AND archived_at IS NULL AND title != ''
               AND lower(title) LIKE '%' || ?1 || '%'
             ORDER BY lower(title) LIKE ?1 || '%' DESC, updated_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![q, limit as i64, vault.as_bytes()], |r| {
            Ok(NoteRef {
                id: uuid_col(r, 0)?,
                title: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The knowledge graph of a vault's active notes (text notes and whiteboards).
    pub fn graph(&self, vault: Option<Uuid>, include_orphans: bool) -> Result<Graph> {
        let vault = self.vault_or_default(vault)?;
        let mut nodes: Vec<GraphNode> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, title, pinned,
                    (SELECT group_concat(t.name, char(31)) FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
                     WHERE nt.note_id = notes.id), kind
                 FROM notes WHERE vault_id = ?1 AND cipher IS NULL AND archived_at IS NULL",
            )?;
            stmt.query_map([vault.as_bytes()], |r| {
                let tags: Option<String> = r.get(3)?;
                let kind: String = r.get(4)?;
                Ok(GraphNode {
                    id: uuid_col(r, 0)?,
                    kind: NoteKind::parse(&kind),
                    title: r.get(1)?,
                    pinned: r.get(2)?,
                    tags: tags
                        .map(|t| t.split('\u{1f}').map(str::to_owned).collect())
                        .unwrap_or_default(),
                    degree: 0,
                })
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let edges: HashSet<GraphEdge> = {
            let mut stmt = self.conn.prepare(
                "SELECT l.source_id, t.id FROM links l
                 JOIN notes s ON s.id = l.source_id AND s.archived_at IS NULL AND s.vault_id = ?1
                 JOIN notes t ON lower(t.title) = l.target_title AND t.vault_id = ?1
                    AND t.cipher IS NULL AND t.archived_at IS NULL AND t.id != l.source_id",
            )?;
            stmt.query_map([vault.as_bytes()], |r| {
                Ok(GraphEdge {
                    source: uuid_col(r, 0)?,
                    target: uuid_col(r, 1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let mut degree: HashMap<Uuid, usize> = HashMap::new();
        for e in &edges {
            *degree.entry(e.source).or_default() += 1;
            *degree.entry(e.target).or_default() += 1;
        }
        for n in &mut nodes {
            n.degree = degree.get(&n.id).copied().unwrap_or(0);
        }
        if !include_orphans {
            nodes.retain(|n| n.degree > 0);
        }
        let mut edges: Vec<GraphEdge> = edges.into_iter().collect();
        edges.sort_by_key(|e| (e.source, e.target));
        Ok(Graph { nodes, edges })
    }
}
