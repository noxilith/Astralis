//! Reusable note templates with `{{placeholders}}`.

use chrono::{DateTime, Local};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notes::{NewNote, Note};
use crate::store::Store;
use crate::util::{Timestamp, now_ms, uuid_col};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: Uuid,
    pub name: String,
    /// Title pattern for notes created from this template.
    pub title: String,
    pub body: String,
    pub builtin: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplateInput {
    pub name: String,
    pub title: String,
    pub body: String,
}

const BUILTINS: &[(&str, &str, &str)] = &[
    (
        "Meeting Notes",
        "Meeting — {{date}}",
        "**Date:** {{weekday}}, {{date}} {{time}}\n**Attendees:** \n\n## Agenda\n- \n\n## Notes\n\n## Decisions\n- \n\n## Action Items\n- [ ] \n",
    ),
    (
        "Daily Journal",
        "Journal — {{date}}",
        "# {{weekday}}, {{date}}\n\n## Grateful for\n- \n\n## Today's focus\n- \n\n## Notes\n\n## Reflection\n",
    ),
    (
        "Project Plan",
        "{{title}}",
        "## Goal\n\n## Scope\n- In: \n- Out: \n\n## Milestones\n- [ ] \n\n## Risks\n- \n\n## Links\n- \n",
    ),
    (
        "Weekly Review",
        "Week of {{date}}",
        "## Wins\n- \n\n## Didn't go well\n- \n\n## Next week\n- [ ] \n\n## Open questions\n- \n",
    ),
];

/// Expands `{{date}}`, `{{time}}`, `{{datetime}}`, `{{weekday}}`, `{{year}}` and `{{title}}`.
pub fn render_template(text: &str, title: &str, at: DateTime<Local>) -> String {
    text.replace("{{date}}", &at.format("%Y-%m-%d").to_string())
        .replace("{{time}}", &at.format("%H:%M").to_string())
        .replace("{{datetime}}", &at.format("%Y-%m-%d %H:%M").to_string())
        .replace("{{weekday}}", &at.format("%A").to_string())
        .replace("{{year}}", &at.format("%Y").to_string())
        .replace("{{title}}", title)
}

fn row_to_template(r: &rusqlite::Row<'_>) -> rusqlite::Result<Template> {
    Ok(Template {
        id: uuid_col(r, 0)?,
        name: r.get(1)?,
        title: r.get(2)?,
        body: r.get(3)?,
        builtin: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

const COLS: &str = "id, name, title, body, builtin, created_at, updated_at";

impl Store {
    pub(crate) fn seed_builtin_templates(&self) -> Result<()> {
        if self.get_setting("templates_seeded")?.is_some() {
            return Ok(());
        }
        let now = now_ms();
        for (name, title, body) in BUILTINS {
            self.conn.execute(
                "INSERT INTO templates(id, name, title, body, builtin, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
                params![Uuid::now_v7().as_bytes(), name, title, body, now],
            )?;
        }
        self.set_setting("templates_seeded", "1")
    }

    pub fn list_templates(&self) -> Result<Vec<Template>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM templates ORDER BY builtin DESC, name"
        ))?;
        let rows = stmt.query_map([], row_to_template)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_template(&self, id: Uuid) -> Result<Template> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM templates WHERE id = ?1"),
                [id.as_bytes()],
                row_to_template,
            )
            .optional()?
            .ok_or(Error::NotFound("template", id))
    }

    pub fn create_template(&self, input: TemplateInput) -> Result<Template> {
        if input.name.trim().is_empty() {
            return Err(Error::invalid("template needs a name"));
        }
        let id = Uuid::now_v7();
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO templates(id, name, title, body, builtin, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
            params![
                id.as_bytes(),
                input.name.trim(),
                input.title,
                input.body,
                now
            ],
        )?;
        self.get_template(id)
    }

    pub fn update_template(&self, id: Uuid, input: TemplateInput) -> Result<Template> {
        let n = self.conn.execute(
            "UPDATE templates SET name = ?2, title = ?3, body = ?4, updated_at = ?5 WHERE id = ?1",
            params![
                id.as_bytes(),
                input.name.trim(),
                input.title,
                input.body,
                now_ms()
            ],
        )?;
        if n == 0 {
            return Err(Error::NotFound("template", id));
        }
        self.get_template(id)
    }

    pub fn delete_template(&self, id: Uuid) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM templates WHERE id = ?1", [id.as_bytes()])?
            == 0
        {
            return Err(Error::NotFound("template", id));
        }
        Ok(())
    }

    /// Saves an existing note's content as a new template.
    pub fn template_from_note(&self, note_id: Uuid, name: &str) -> Result<Template> {
        let note = self.get_note(note_id)?;
        self.create_template(TemplateInput {
            name: name.to_owned(),
            title: note.title,
            body: note.body,
        })
    }

    /// Creates a note from a template. `title` fills `{{title}}` (and the title if the pattern is empty).
    pub fn note_from_template(
        &self,
        template_id: Uuid,
        title: Option<String>,
        vault_id: Option<Uuid>,
    ) -> Result<Note> {
        let t = self.get_template(template_id)?;
        let given = title.unwrap_or_default();
        let now = Local::now();
        let mut note_title = render_template(&t.title, &given, now);
        if note_title.trim().is_empty() {
            note_title = given.clone();
        }
        let body = render_template(&t.body, &given, now);
        self.create_note(NewNote {
            title: note_title,
            body,
            vault_id,
            ..Default::default()
        })
    }
}
