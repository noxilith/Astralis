//! Calendar events and a combined calendar view of events and tasks.

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::tasks::{Priority, TaskFilter, TaskStatus};
use crate::util::{Timestamp, now_ms, uuid_col};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_at: Timestamp,
    pub end_at: Timestamp,
    pub all_day: bool,
    pub remind_at: Option<Timestamp>,
    pub reminded: bool,
    pub color: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EventInput {
    pub title: String,
    pub description: String,
    pub location: String,
    pub start_at: Timestamp,
    pub end_at: Timestamp,
    pub all_day: bool,
    pub remind_at: Option<Timestamp>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalendarKind {
    Event,
    Task,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarItem {
    pub kind: CalendarKind,
    pub id: Uuid,
    pub title: String,
    pub start: Timestamp,
    pub end: Option<Timestamp>,
    pub all_day: bool,
    pub done: bool,
    pub priority: Option<Priority>,
    pub color: Option<String>,
    /// A future occurrence of a recurring task (not a stored row).
    pub projected: bool,
}

const COLS: &str = "id, title, description, location, start_at, end_at, all_day, remind_at, reminded, color, created_at, updated_at";

fn row_to_event(r: &Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: uuid_col(r, 0)?,
        title: r.get(1)?,
        description: r.get(2)?,
        location: r.get(3)?,
        start_at: r.get(4)?,
        end_at: r.get(5)?,
        all_day: r.get(6)?,
        remind_at: r.get(7)?,
        reminded: r.get(8)?,
        color: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn validate(input: &EventInput) -> Result<()> {
    if input.title.trim().is_empty() {
        return Err(Error::invalid("event needs a title"));
    }
    if input.end_at < input.start_at {
        return Err(Error::invalid("event ends before it starts"));
    }
    Ok(())
}

impl Store {
    pub fn create_event(&self, input: EventInput) -> Result<Event> {
        validate(&input)?;
        let id = Uuid::now_v7();
        let now = now_ms();
        self.conn.execute(
            &format!("INSERT INTO events ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?10)"),
            params![
                id.as_bytes(),
                input.title.trim(),
                input.description,
                input.location,
                input.start_at,
                input.end_at,
                input.all_day,
                input.remind_at,
                input.color,
                now
            ],
        )?;
        self.get_event(id)
    }

    pub fn get_event(&self, id: Uuid) -> Result<Event> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM events WHERE id = ?1"),
                [id.as_bytes()],
                row_to_event,
            )
            .optional()?
            .ok_or(Error::NotFound("event", id))
    }

    pub fn update_event(&self, id: Uuid, input: EventInput) -> Result<Event> {
        validate(&input)?;
        let old = self.get_event(id)?;
        let reminded = old.reminded && old.remind_at == input.remind_at;
        self.conn.execute(
            "UPDATE events SET title = ?2, description = ?3, location = ?4, start_at = ?5, end_at = ?6,
                all_day = ?7, remind_at = ?8, reminded = ?9, color = ?10, updated_at = ?11 WHERE id = ?1",
            params![
                id.as_bytes(),
                input.title.trim(),
                input.description,
                input.location,
                input.start_at,
                input.end_at,
                input.all_day,
                input.remind_at,
                reminded,
                input.color,
                now_ms()
            ],
        )?;
        self.get_event(id)
    }

    pub fn delete_event(&self, id: Uuid) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM events WHERE id = ?1", [id.as_bytes()])?
            == 0
        {
            return Err(Error::NotFound("event", id));
        }
        Ok(())
    }

    /// Everything on the calendar between `start` (inclusive) and `end` (exclusive):
    /// overlapping events, tasks due in the range, and projected recurring-task occurrences.
    pub fn calendar(&self, start: Timestamp, end: Timestamp) -> Result<Vec<CalendarItem>> {
        let mut items = Vec::new();
        {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {COLS} FROM events WHERE start_at < ?2 AND end_at >= ?1 ORDER BY start_at"
            ))?;
            for e in stmt.query_map(params![start, end], row_to_event)? {
                let e = e?;
                items.push(CalendarItem {
                    kind: CalendarKind::Event,
                    id: e.id,
                    title: e.title,
                    start: e.start_at,
                    end: Some(e.end_at),
                    all_day: e.all_day,
                    done: false,
                    priority: None,
                    color: e.color,
                    projected: false,
                });
            }
        }
        let tasks = self.list_tasks(&TaskFilter {
            include_done: true,
            due_before: Some(end),
            ..Default::default()
        })?;
        for t in tasks {
            let Some(due) = t.due_at else { continue };
            let done = t.status == TaskStatus::Done;
            if due >= start {
                items.push(CalendarItem {
                    kind: CalendarKind::Task,
                    id: t.id,
                    title: t.title.clone(),
                    start: due,
                    end: None,
                    all_day: t.all_day,
                    done,
                    priority: Some(t.priority),
                    color: None,
                    projected: false,
                });
            }
            if let (Some(r), false) = (&t.recurrence, done) {
                let mut next = r.step(due);
                let mut count = 0;
                while next < end && count < 500 && r.until.is_none_or(|u| next <= u) {
                    if next >= start {
                        items.push(CalendarItem {
                            kind: CalendarKind::Task,
                            id: t.id,
                            title: t.title.clone(),
                            start: next,
                            end: None,
                            all_day: t.all_day,
                            done: false,
                            priority: Some(t.priority),
                            color: None,
                            projected: true,
                        });
                    }
                    next = r.step(next);
                    count += 1;
                }
            }
        }
        items.sort_by_key(|i| (i.start, i.kind == CalendarKind::Task));
        Ok(items)
    }
}
