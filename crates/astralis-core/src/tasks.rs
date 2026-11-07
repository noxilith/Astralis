//! To-dos with due dates, priorities, recurrence and reminders.

use chrono::{Duration, Local, LocalResult, Months, NaiveDateTime, TimeZone};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::util::{Timestamp, double_option, now_ms, opt_bytes, opt_uuid_col, uuid_col};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[default]
    Todo,
    Doing,
    Done,
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Todo => "todo",
            TaskStatus::Doing => "doing",
            TaskStatus::Done => "done",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "doing" => TaskStatus::Doing,
            "done" => TaskStatus::Done,
            _ => TaskStatus::Todo,
        }
    }
}

/// 0 = none, 1 = low, 2 = medium, 3 = high, 4 = urgent.
pub type Priority = u8;
pub const MAX_PRIORITY: Priority = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recurrence {
    pub freq: Frequency,
    /// Every `interval` days/weeks/months/years. Minimum 1.
    #[serde(default = "one")]
    pub interval: u32,
    /// Stop repeating after this moment.
    #[serde(default)]
    pub until: Option<Timestamp>,
}

fn one() -> u32 {
    1
}

impl Recurrence {
    /// The occurrence after `from`, keeping the same local wall-clock time (DST-safe).
    pub fn step(&self, from: Timestamp) -> Timestamp {
        let n = self.interval.max(1);
        let local = match Local.timestamp_millis_opt(from) {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t,
            LocalResult::None => return from + 86_400_000 * n as i64,
        };
        let naive: NaiveDateTime = local.naive_local();
        let next = match self.freq {
            Frequency::Daily => naive.checked_add_signed(Duration::days(n as i64)),
            Frequency::Weekly => naive.checked_add_signed(Duration::weeks(n as i64)),
            Frequency::Monthly => add_months_clamped(naive, n),
            Frequency::Yearly => add_months_clamped(naive, n * 12),
        };
        let Some(next) = next else {
            return from + 86_400_000;
        };
        match Local.from_local_datetime(&next) {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.timestamp_millis(),
            // Skipped by a DST jump: shift forward an hour.
            LocalResult::None => Local
                .from_local_datetime(&(next + Duration::hours(1)))
                .earliest()
                .map(|t| t.timestamp_millis())
                .unwrap_or(from + 86_400_000),
        }
    }

    /// First occurrence after `due` that is in the future (relative to `now`).
    pub fn next_after(&self, due: Timestamp, now: Timestamp) -> Timestamp {
        let mut next = self.step(due);
        let mut guard = 0;
        while next <= now && guard < 100_000 {
            next = self.step(next);
            guard += 1;
        }
        next
    }
}

/// Adds months; chrono clamps the day (Jan 31 + 1 month = Feb 28/29).
fn add_months_clamped(t: NaiveDateTime, months: u32) -> Option<NaiveDateTime> {
    t.checked_add_months(Months::new(months))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: Uuid,
    pub title: String,
    pub notes: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub due_at: Option<Timestamp>,
    pub all_day: bool,
    pub remind_at: Option<Timestamp>,
    pub reminded: bool,
    pub recurrence: Option<Recurrence>,
    pub planner_id: Option<Uuid>,
    pub note_id: Option<Uuid>,
    pub completed_at: Option<Timestamp>,
    pub completion_count: i64,
    pub position: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewTask {
    pub title: String,
    pub notes: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub due_at: Option<Timestamp>,
    pub all_day: bool,
    pub remind_at: Option<Timestamp>,
    pub recurrence: Option<Recurrence>,
    pub planner_id: Option<Uuid>,
    pub note_id: Option<Uuid>,
}

/// Partial update. For nullable fields, `Some(None)` clears the value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<Priority>,
    #[serde(deserialize_with = "double_option")]
    pub due_at: Option<Option<Timestamp>>,
    pub all_day: Option<bool>,
    #[serde(deserialize_with = "double_option")]
    pub remind_at: Option<Option<Timestamp>>,
    #[serde(deserialize_with = "double_option")]
    pub recurrence: Option<Option<Recurrence>>,
    #[serde(deserialize_with = "double_option")]
    pub planner_id: Option<Option<Uuid>>,
    #[serde(deserialize_with = "double_option")]
    pub note_id: Option<Option<Uuid>>,
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskFilter {
    pub include_done: bool,
    pub planner_id: Option<Uuid>,
    pub note_id: Option<Uuid>,
    /// Only tasks with a due date before this moment (e.g. end of today).
    pub due_before: Option<Timestamp>,
    pub due_after: Option<Timestamp>,
    /// Only tasks with no due date.
    pub no_due_date: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderKind {
    Task,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub kind: ReminderKind,
    pub id: Uuid,
    pub title: String,
    pub remind_at: Timestamp,
    /// When the task is due / the event starts.
    pub at: Option<Timestamp>,
}

const COLS: &str =
    "id, title, notes, status, priority, due_at, all_day, remind_at, reminded, recurrence,
    planner_id, note_id, completed_at, completion_count, position, created_at, updated_at";

fn row_to_task(r: &Row<'_>) -> rusqlite::Result<Task> {
    let status: String = r.get(3)?;
    let recurrence: Option<String> = r.get(9)?;
    Ok(Task {
        id: uuid_col(r, 0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        status: TaskStatus::parse(&status),
        priority: r.get(4)?,
        due_at: r.get(5)?,
        all_day: r.get(6)?,
        remind_at: r.get(7)?,
        reminded: r.get(8)?,
        recurrence: recurrence.and_then(|s| serde_json::from_str(&s).ok()),
        planner_id: opt_uuid_col(r, 10)?,
        note_id: opt_uuid_col(r, 11)?,
        completed_at: r.get(12)?,
        completion_count: r.get(13)?,
        position: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

fn check_priority(p: Priority) -> Result<Priority> {
    if p > MAX_PRIORITY {
        return Err(Error::invalid(format!(
            "priority must be 0..={MAX_PRIORITY}"
        )));
    }
    Ok(p)
}

impl Store {
    pub fn create_task(&self, new: NewTask) -> Result<Task> {
        if new.title.trim().is_empty() {
            return Err(Error::invalid("task needs a title"));
        }
        if new.recurrence.is_some() && new.due_at.is_none() {
            return Err(Error::invalid("recurring tasks need a due date"));
        }
        let id = Uuid::now_v7();
        let now = now_ms();
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM tasks",
            [],
            |r| r.get(0),
        )?;
        let recurrence = new
            .recurrence
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let done = new.status == TaskStatus::Done;
        self.conn.execute(
            &format!(
                "INSERT INTO tasks ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)"
            ),
            params![
                id.as_bytes(),
                new.title.trim(),
                new.notes,
                new.status.as_str(),
                check_priority(new.priority)?,
                new.due_at,
                new.all_day,
                new.remind_at,
                recurrence,
                opt_bytes(new.planner_id),
                opt_bytes(new.note_id),
                done.then_some(now),
                done as i64,
                position,
                now
            ],
        )?;
        self.get_task(id)
    }

    pub fn get_task(&self, id: Uuid) -> Result<Task> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM tasks WHERE id = ?1"),
                [id.as_bytes()],
                row_to_task,
            )
            .optional()?
            .ok_or(Error::NotFound("task", id))
    }

    pub fn update_task(&self, id: Uuid, patch: TaskPatch) -> Result<Task> {
        let mut t = self.get_task(id)?;
        let completing = patch.status == Some(TaskStatus::Done) && t.status != TaskStatus::Done;
        if let Some(v) = patch.title {
            if v.trim().is_empty() {
                return Err(Error::invalid("task needs a title"));
            }
            t.title = v.trim().to_owned();
        }
        if let Some(v) = patch.notes {
            t.notes = v;
        }
        if let Some(v) = patch.priority {
            t.priority = check_priority(v)?;
        }
        if let Some(v) = patch.due_at {
            t.due_at = v;
        }
        if let Some(v) = patch.all_day {
            t.all_day = v;
        }
        if let Some(v) = patch.remind_at {
            if v != t.remind_at {
                t.reminded = false;
            }
            t.remind_at = v;
        }
        if let Some(v) = patch.recurrence {
            t.recurrence = v;
        }
        if let Some(v) = patch.planner_id {
            t.planner_id = v;
        }
        if let Some(v) = patch.note_id {
            t.note_id = v;
        }
        if let Some(v) = patch.position {
            t.position = v;
        }
        if t.recurrence.is_some() && t.due_at.is_none() {
            return Err(Error::invalid("recurring tasks need a due date"));
        }
        if let Some(s) = patch.status
            && s != TaskStatus::Done
        {
            t.status = s;
            t.completed_at = None;
        }
        self.save_task(&t)?;
        if completing {
            self.complete_task(id)
        } else {
            self.get_task(id)
        }
    }

    fn save_task(&self, t: &Task) -> Result<()> {
        let recurrence = t
            .recurrence
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn.execute(
            "UPDATE tasks SET title = ?2, notes = ?3, status = ?4, priority = ?5, due_at = ?6, all_day = ?7,
                remind_at = ?8, reminded = ?9, recurrence = ?10, planner_id = ?11, note_id = ?12,
                completed_at = ?13, completion_count = ?14, position = ?15, updated_at = ?16
             WHERE id = ?1",
            params![
                t.id.as_bytes(),
                t.title,
                t.notes,
                t.status.as_str(),
                t.priority,
                t.due_at,
                t.all_day,
                t.remind_at,
                t.reminded,
                recurrence,
                opt_bytes(t.planner_id),
                opt_bytes(t.note_id),
                t.completed_at,
                t.completion_count,
                t.position,
                now_ms()
            ],
        )?;
        Ok(())
    }

    /// Marks a task done. Recurring tasks instead roll forward to their next occurrence
    /// (shifting the reminder by the same amount) until their `until` date passes.
    pub fn complete_task(&self, id: Uuid) -> Result<Task> {
        self.complete_task_at(id, now_ms())
    }

    pub(crate) fn complete_task_at(&self, id: Uuid, now: Timestamp) -> Result<Task> {
        let mut t = self.get_task(id)?;
        t.completion_count += 1;
        let next = match (&t.recurrence, t.due_at) {
            (Some(r), Some(due)) => {
                let next = r.next_after(due, now);
                r.until.is_none_or(|u| next <= u).then_some((due, next))
            }
            _ => None,
        };
        match next {
            Some((due, next)) => {
                t.due_at = Some(next);
                t.remind_at = t.remind_at.map(|rem| rem + (next - due));
                t.reminded = false;
                t.status = TaskStatus::Todo;
                t.completed_at = Some(now);
            }
            None => {
                t.status = TaskStatus::Done;
                t.completed_at = Some(now);
            }
        }
        self.save_task(&t)?;
        self.get_task(id)
    }

    pub fn reopen_task(&self, id: Uuid) -> Result<Task> {
        self.update_task(
            id,
            TaskPatch {
                status: Some(TaskStatus::Todo),
                ..Default::default()
            },
        )
    }

    pub fn delete_task(&self, id: Uuid) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM tasks WHERE id = ?1", [id.as_bytes()])?
            == 0
        {
            return Err(Error::NotFound("task", id));
        }
        Ok(())
    }

    /// Open tasks first, then by priority (high first), due date (soonest first), manual position.
    pub fn list_tasks(&self, f: &TaskFilter) -> Result<Vec<Task>> {
        let mut clauses = vec!["1 = 1".to_owned()];
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !f.include_done {
            clauses.push("status != 'done'".into());
        }
        if let Some(p) = f.planner_id {
            args.push(Box::new(p.as_bytes().to_vec()));
            clauses.push(format!("planner_id = ?{}", args.len()));
        }
        if let Some(n) = f.note_id {
            args.push(Box::new(n.as_bytes().to_vec()));
            clauses.push(format!("note_id = ?{}", args.len()));
        }
        if let Some(t) = f.due_before {
            args.push(Box::new(t));
            clauses.push(format!("due_at IS NOT NULL AND due_at < ?{}", args.len()));
        }
        if let Some(t) = f.due_after {
            args.push(Box::new(t));
            clauses.push(format!("due_at IS NOT NULL AND due_at >= ?{}", args.len()));
        }
        if f.no_due_date {
            clauses.push("due_at IS NULL".into());
        }
        let sql = format!(
            "SELECT {COLS} FROM tasks WHERE {} ORDER BY status = 'done', priority DESC,
             due_at IS NULL, due_at, position",
            clauses.join(" AND ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---- reminders ----

    /// Task and event reminders whose time has come and that haven't fired yet.
    pub fn due_reminders(&self, now: Timestamp) -> Result<Vec<Reminder>> {
        let mut out = Vec::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT id, title, remind_at, due_at FROM tasks
                 WHERE reminded = 0 AND remind_at IS NOT NULL AND remind_at <= ?1 AND status != 'done'",
            )?;
            let rows = stmt.query_map([now], |r| {
                Ok(Reminder {
                    kind: ReminderKind::Task,
                    id: uuid_col(r, 0)?,
                    title: r.get(1)?,
                    remind_at: r.get(2)?,
                    at: r.get(3)?,
                })
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        {
            let mut stmt = self.conn.prepare(
                "SELECT id, title, remind_at, start_at FROM events
                 WHERE reminded = 0 AND remind_at IS NOT NULL AND remind_at <= ?1",
            )?;
            let rows = stmt.query_map([now], |r| {
                Ok(Reminder {
                    kind: ReminderKind::Event,
                    id: uuid_col(r, 0)?,
                    title: r.get(1)?,
                    remind_at: r.get(2)?,
                    at: r.get(3)?,
                })
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        out.sort_by_key(|r| r.remind_at);
        Ok(out)
    }

    pub fn mark_reminded(&self, kind: ReminderKind, id: Uuid) -> Result<()> {
        let table = match kind {
            ReminderKind::Task => "tasks",
            ReminderKind::Event => "events",
        };
        self.conn.execute(
            &format!("UPDATE {table} SET reminded = 1 WHERE id = ?1"),
            [id.as_bytes()],
        )?;
        Ok(())
    }

    pub fn snooze_reminder(&self, kind: ReminderKind, id: Uuid, until: Timestamp) -> Result<()> {
        let table = match kind {
            ReminderKind::Task => "tasks",
            ReminderKind::Event => "events",
        };
        let n = self.conn.execute(
            &format!("UPDATE {table} SET remind_at = ?2, reminded = 0 WHERE id = ?1"),
            params![id.as_bytes(), until],
        )?;
        if n == 0 {
            return Err(Error::NotFound("reminder", id));
        }
        Ok(())
    }
}
