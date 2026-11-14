//! The user's tasks, planners and calendar for AI agents (through the Brain's MCP server): make a
//! planner for a project with its tasks, add to-dos, put appointments on the calendar, tick things
//! off. Like the notes tools, answers are short plain text and things are referred to by a
//! `[handle]` (the end of the id) or by title. Times are the user's local time.

use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use uuid::Uuid;

use crate::agent_notes::AgentAccess;
use crate::calendar::{CalendarKind, EventInput};
use crate::error::{Error, Result};
use crate::planners::{Planner, PlannerInput};
use crate::store::Store;
use crate::tasks::{Frequency, NewTask, Recurrence, Task, TaskFilter, TaskPatch, TaskStatus};
use crate::util::Timestamp;

/// Settings key: what agents may do with tasks and the calendar ("edit" by default).
pub const PLAN_ACCESS_KEY: &str = "agents.plan";

fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

fn handle(id: Uuid) -> String {
    crate::agent_notes::handle(id)
}

/// A moment the agent wrote: `2025-03-04`, `2025-03-04 14:30` or `2025-03-04T14:30` (the user's
/// local time), or a full RFC 3339 time with an offset. Returns whether a time of day was given.
pub fn parse_when(s: &str) -> Result<(Timestamp, bool)> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok((t.timestamp_millis(), true));
    }
    let local = |n: NaiveDateTime| match Local.from_local_datetime(&n) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t.timestamp_millis()),
        LocalResult::None => None,
    };
    for f in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return local(n).map(|t| (t, true)).ok_or_else(|| invalid(format!("\"{s}\" doesn't exist in the user's time zone (daylight saving gap)")));
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return local(d.and_time(NaiveTime::MIN)).map(|t| (t, false)).ok_or_else(|| invalid(format!("couldn't read \"{s}\"")));
    }
    Err(invalid(format!("couldn't read the time \"{s}\"; write it as YYYY-MM-DD or YYYY-MM-DD HH:MM (local time)")))
}

fn at(ts: Timestamp) -> DateTime<Local> {
    Local.timestamp_millis_opt(ts).single().unwrap_or_else(Local::now)
}

fn show(ts: Timestamp, all_day: bool) -> String {
    let t = at(ts);
    if all_day { t.format("%a %Y-%m-%d").to_string() } else { t.format("%a %Y-%m-%d %H:%M").to_string() }
}

/// End of the local day containing `ts` (23:59), how the app stores an all-day due time.
fn end_of_day(ts: Timestamp) -> Timestamp {
    let d = at(ts).date_naive();
    let n = d.and_hms_opt(23, 59, 0).unwrap_or_default();
    match Local.from_local_datetime(&n) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.timestamp_millis(),
        LocalResult::None => ts + 86_340_000,
    }
}

fn priority_of(s: &str) -> Result<u8> {
    Ok(match s.trim().to_lowercase().as_str() {
        "" | "none" | "0" => 0,
        "low" | "1" => 1,
        "medium" | "normal" | "2" => 2,
        "high" | "3" => 3,
        "urgent" | "4" => 4,
        other => return Err(invalid(format!("priority \"{other}\": use none, low, medium, high or urgent"))),
    })
}

fn priority_name(p: u8) -> &'static str {
    ["", "low", "medium", "high", "urgent"].get(p as usize).copied().unwrap_or("")
}

fn recurrence_of(s: &str) -> Result<Option<Recurrence>> {
    let s = s.trim().to_lowercase();
    if s.is_empty() || s == "none" || s == "never" {
        return Ok(None);
    }
    let (interval, word) = match s.split_once(' ') {
        Some((n, w)) if n.parse::<u32>().is_ok() => (n.parse().unwrap_or(1), w.trim().to_string()),
        _ => (1, s.trim_start_matches("every ").to_string()),
    };
    let freq = match word.trim_end_matches('s') {
        "day" | "daily" => Frequency::Daily,
        "week" | "weekly" => Frequency::Weekly,
        "month" | "monthly" => Frequency::Monthly,
        "year" | "yearly" => Frequency::Yearly,
        _ => return Err(invalid("repeat: use daily, weekly, monthly or yearly (optionally \"2 weeks\"), or \"none\"")),
    };
    Ok(Some(Recurrence { freq, interval: interval.max(1), until: None }))
}

fn repeat_text(r: &Recurrence) -> String {
    let w = match r.freq {
        Frequency::Daily => "day",
        Frequency::Weekly => "week",
        Frequency::Monthly => "month",
        Frequency::Yearly => "year",
    };
    if r.interval > 1 { format!("every {} {w}s", r.interval) } else { format!("every {w}") }
}

/// A task an agent asks for.
#[derive(Debug, Clone, Default)]
pub struct AgentTask {
    pub title: String,
    pub notes: Option<String>,
    /// `todo` (default), `doing` or `done`.
    pub status: Option<String>,
    /// none, low, medium, high, urgent.
    pub priority: Option<String>,
    /// Due date (all day) or date and time.
    pub due: Option<String>,
    /// When to remind: a time, in the same format as `due`.
    pub remind: Option<String>,
    /// daily, weekly, monthly, yearly, "2 weeks"…
    pub repeat: Option<String>,
    /// Planner by name or handle.
    pub planner: Option<String>,
}

/// A change to a task; `None` leaves a field alone. An empty `due`/`remind`/`planner`/`repeat` clears it.
#[derive(Debug, Clone, Default)]
pub struct AgentTaskEdit {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub due: Option<String>,
    pub remind: Option<String>,
    pub repeat: Option<String>,
    pub planner: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentEvent {
    pub title: String,
    pub start: String,
    /// Defaults to an hour after the start (or the end of the day for an all-day event).
    pub end: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// Remind this many minutes before it starts.
    pub remind_minutes: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentEventEdit {
    pub title: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// Minutes before the start; a negative number removes the reminder.
    pub remind_minutes: Option<i64>,
}

impl Store {
    pub fn agent_plan_access(&self) -> Result<AgentAccess> {
        Ok(self.get_setting(PLAN_ACCESS_KEY)?.and_then(|s| AgentAccess::parse(&s)).unwrap_or(AgentAccess::Edit))
    }

    pub fn set_agent_plan_access(&self, access: AgentAccess) -> Result<()> {
        self.set_setting(PLAN_ACCESS_KEY, access.as_str())
    }

    // ---- finding things by [handle] or title ----

    fn find_planner(&self, which: &str) -> Result<Planner> {
        let which = which.trim().trim_matches(['[', ']']);
        let all = self.list_planners()?;
        let lower = which.to_lowercase();
        let hits: Vec<&Planner> = all.iter().filter(|p| handle(p.id) == lower || p.id.to_string() == which).collect();
        let hits = if hits.is_empty() { all.iter().filter(|p| p.name.to_lowercase() == lower).collect() } else { hits };
        match hits.as_slice() {
            [p] => Ok((*p).clone()),
            [] => Err(invalid(format!("no planner \"{which}\"; planners: {}", all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")))),
            _ => Err(invalid(format!("several planners are called \"{which}\"; use a [handle]"))),
        }
    }

    fn find_task(&self, which: &str) -> Result<Task> {
        let which = which.trim().trim_matches(['[', ']']);
        let all = self.list_tasks(&TaskFilter { include_done: true, ..Default::default() })?;
        let lower = which.to_lowercase();
        let by_handle: Vec<&Task> = all.iter().filter(|t| handle(t.id) == lower || t.id.to_string() == which).collect();
        let hits = if by_handle.is_empty() { all.iter().filter(|t| t.title.to_lowercase() == lower).collect() } else { by_handle };
        match hits.as_slice() {
            [t] => Ok((*t).clone()),
            [] => Err(invalid(format!("no task \"{which}\" (see tasks_list for [handles])"))),
            _ => Err(invalid(format!("several tasks are called \"{which}\"; use a [handle] from tasks_list"))),
        }
    }

    fn find_event(&self, which: &str) -> Result<crate::calendar::Event> {
        let which = which.trim().trim_matches(['[', ']']);
        let all = self.calendar(0, i64::MAX / 4)?;
        let lower = which.to_lowercase();
        let mut hits: Vec<Uuid> = Vec::new();
        for i in all.iter().filter(|i| i.kind == CalendarKind::Event) {
            if (handle(i.id) == lower || i.id.to_string() == which || i.title.to_lowercase() == lower) && !hits.contains(&i.id) {
                hits.push(i.id);
            }
        }
        match hits.as_slice() {
            [id] => self.get_event(*id),
            [] => Err(invalid(format!("no calendar event \"{which}\" (see calendar_list for [handles])"))),
            _ => Err(invalid(format!("several events are called \"{which}\"; use a [handle] from calendar_list"))),
        }
    }

    fn planner_for(&self, name: Option<&str>) -> Result<Option<Option<Uuid>>> {
        match name.map(str::trim) {
            None => Ok(None),
            Some("") => Ok(Some(None)),
            Some(n) => Ok(Some(Some(self.find_planner(n)?.id))),
        }
    }

    // ---- reading ----

    /// The current local date and time (agents don't otherwise know "today").
    pub fn agent_now(&self) -> String {
        Local::now().format("%A %Y-%m-%d %H:%M (local time)").to_string()
    }

    pub fn agent_planners(&self) -> Result<String> {
        let list = self.list_planners()?;
        let mut out = format!("Now: {}\n", self.agent_now());
        if list.is_empty() {
            out.push_str("No planners yet.");
            return Ok(out);
        }
        for p in list {
            out.push_str(&format!("[{}] {} — {}/{} done{}\n", handle(p.id), p.name, p.done_count, p.task_count, if p.description.is_empty() { String::new() } else { format!(" — {}", p.description) }));
        }
        Ok(out.trim_end().to_string())
    }

    fn task_line(&self, t: &Task, planners: &[Planner]) -> String {
        let mut line = format!("[{}] {}{}", handle(t.id), if t.status == TaskStatus::Done { "✓ " } else { "" }, t.title);
        let mut bits: Vec<String> = Vec::new();
        if t.status == TaskStatus::Doing {
            bits.push("doing".into());
        }
        if t.priority > 0 {
            bits.push(priority_name(t.priority).into());
        }
        if let Some(d) = t.due_at {
            bits.push(format!("due {}", show(d, t.all_day)));
        }
        if let Some(r) = &t.recurrence {
            bits.push(repeat_text(r));
        }
        if let Some(p) = t.planner_id.and_then(|id| planners.iter().find(|p| p.id == id)) {
            bits.push(format!("in {}", p.name));
        }
        if !bits.is_empty() {
            line.push_str(&format!(" — {}", bits.join(", ")));
        }
        if !t.notes.trim().is_empty() {
            let n: String = t.notes.split_whitespace().collect::<Vec<_>>().join(" ");
            line.push_str(&format!("\n    {}", if n.chars().count() > 160 { format!("{}…", n.chars().take(159).collect::<String>()) } else { n }));
        }
        line
    }

    pub fn agent_tasks(&self, planner: Option<&str>, include_done: bool, due_before: Option<&str>, limit: usize) -> Result<String> {
        let planners = self.list_planners()?;
        let mut f = TaskFilter { include_done, ..Default::default() };
        if let Some(p) = planner.filter(|p| !p.trim().is_empty()) {
            f.planner_id = Some(self.find_planner(p)?.id);
        }
        if let Some(d) = due_before.filter(|d| !d.trim().is_empty()) {
            let (t, timed) = parse_when(d)?;
            f.due_before = Some(if timed { t } else { end_of_day(t) + 60_000 });
        }
        let tasks = self.list_tasks(&f)?;
        if tasks.is_empty() {
            return Ok("No tasks.".into());
        }
        let total = tasks.len();
        let mut out: Vec<String> = tasks.iter().take(limit).map(|t| self.task_line(t, &planners)).collect();
        if total > limit {
            out.push(format!("+{} more (narrow with planner or due_before).", total - limit));
        }
        Ok(out.join("\n"))
    }

    pub fn agent_calendar(&self, from: &str, to: &str) -> Result<String> {
        let (start, _) = parse_when(from)?;
        let (end, timed) = parse_when(to)?;
        let end = if timed { end } else { end_of_day(end) + 60_000 };
        if end <= start {
            return Err(invalid("`to` must be after `from`"));
        }
        let items = self.calendar(start, end)?;
        if items.is_empty() {
            return Ok("Nothing on the calendar then.".into());
        }
        let mut out = Vec::new();
        for i in items.iter().take(120) {
            let what = match i.kind {
                CalendarKind::Event => {
                    let e = self.get_event(i.id)?;
                    let mut s = format!("{} [{}] {}", if i.all_day { show(i.start, true) } else { format!("{}–{}", show(i.start, false), at(e.end_at).format("%H:%M")) }, handle(i.id), i.title);
                    if !e.location.is_empty() {
                        s.push_str(&format!(" @ {}", e.location));
                    }
                    s
                }
                CalendarKind::Task => format!("{} task [{}] {}{}", show(i.start, i.all_day), handle(i.id), if i.done { "✓ " } else { "" }, i.title),
            };
            out.push(what);
        }
        Ok(out.join("\n"))
    }

    // ---- planners ----

    /// Makes a planner, optionally with its first tasks in one go.
    pub fn agent_planner_create(&self, name: &str, description: &str, tasks: &[AgentTask]) -> Result<String> {
        // Check every task first so a typo doesn't leave a half-made planner.
        let planned = tasks.iter().map(|t| self.new_task(t, None)).collect::<Result<Vec<_>>>()?;
        let p = self.create_planner(PlannerInput { name: name.into(), description: description.into(), color: None })?;
        for mut nt in planned {
            nt.planner_id = Some(p.id);
            self.create_task(nt)?;
        }
        Ok(format!("Made the planner [{}] {} with {} task{}.", handle(p.id), p.name, tasks.len(), if tasks.len() == 1 { "" } else { "s" }))
    }

    pub fn agent_planner_update(&self, which: &str, name: Option<&str>, description: Option<&str>) -> Result<String> {
        let p = self.find_planner(which)?;
        let p = self.update_planner(
            p.id,
            PlannerInput { name: name.map(str::to_owned).unwrap_or(p.name), description: description.map(str::to_owned).unwrap_or(p.description), color: Some(p.color) },
        )?;
        Ok(format!("Updated [{}] {}.", handle(p.id), p.name))
    }

    /// Deletes a planner. Its tasks stay (without a planner) unless `with_tasks`.
    pub fn agent_planner_delete(&self, which: &str, with_tasks: bool) -> Result<String> {
        let p = self.find_planner(which)?;
        let mut removed = 0;
        if with_tasks {
            for t in self.list_tasks(&TaskFilter { include_done: true, planner_id: Some(p.id), ..Default::default() })? {
                self.delete_task(t.id)?;
                removed += 1;
            }
        }
        self.delete_planner(p.id)?;
        Ok(if with_tasks { format!("Deleted the planner {} and its {removed} tasks.", p.name) } else { format!("Deleted the planner {}; its tasks are kept.", p.name) })
    }

    // ---- tasks ----

    fn new_task(&self, t: &AgentTask, default_planner: Option<Uuid>) -> Result<NewTask> {
        if t.title.trim().is_empty() {
            return Err(invalid("a task needs a title"));
        }
        let (due_at, all_day) = match t.due.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => {
                let (ts, timed) = parse_when(d)?;
                (Some(if timed { ts } else { end_of_day(ts) }), !timed)
            }
            None => (None, false),
        };
        let remind_at = match t.remind.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => Some(parse_when(d)?.0),
            None => None,
        };
        let status = match t.status.as_deref().map(|s| s.trim().to_lowercase()).as_deref() {
            None | Some("") | Some("todo") => TaskStatus::Todo,
            Some("doing") => TaskStatus::Doing,
            Some("done") => TaskStatus::Done,
            Some(o) => return Err(invalid(format!("status \"{o}\": use todo, doing or done"))),
        };
        let planner_id = match self.planner_for(t.planner.as_deref())? {
            Some(p) => p,
            None => default_planner,
        };
        let recurrence = match t.repeat.as_deref() {
            Some(r) => recurrence_of(r)?,
            None => None,
        };
        Ok(NewTask {
            title: t.title.clone(),
            notes: t.notes.clone().unwrap_or_default(),
            status,
            priority: priority_of(t.priority.as_deref().unwrap_or(""))?,
            due_at,
            all_day,
            remind_at,
            recurrence,
            planner_id,
            note_id: None,
        })
    }

    pub fn agent_tasks_add(&self, tasks: &[AgentTask]) -> Result<String> {
        if tasks.is_empty() {
            return Err(invalid("no tasks given"));
        }
        let planned = tasks.iter().map(|t| self.new_task(t, None)).collect::<Result<Vec<_>>>()?;
        let mut out = Vec::new();
        for nt in planned {
            let t = self.create_task(nt)?;
            out.push(format!("[{}] {}", handle(t.id), t.title));
        }
        Ok(format!("Added {}: {}", if out.len() == 1 { "1 task".to_string() } else { format!("{} tasks", out.len()) }, out.join("; ")))
    }

    pub fn agent_task_edit(&self, which: &str, e: &AgentTaskEdit) -> Result<String> {
        let t = self.find_task(which)?;
        let mut patch = TaskPatch { title: e.title.clone(), notes: e.notes.clone(), ..Default::default() };
        if let Some(p) = &e.priority {
            patch.priority = Some(priority_of(p)?);
        }
        if let Some(d) = &e.due {
            if d.trim().is_empty() {
                patch.due_at = Some(None);
            } else {
                let (ts, timed) = parse_when(d)?;
                patch.due_at = Some(Some(if timed { ts } else { end_of_day(ts) }));
                patch.all_day = Some(!timed);
            }
        }
        if let Some(r) = &e.remind {
            patch.remind_at = Some(if r.trim().is_empty() { None } else { Some(parse_when(r)?.0) });
        }
        if let Some(r) = &e.repeat {
            patch.recurrence = Some(recurrence_of(r)?);
        }
        if let Some(p) = self.planner_for(e.planner.as_deref())? {
            patch.planner_id = Some(p);
        }
        // Done goes through complete_task so a repeating task moves to its next date.
        let mut complete = false;
        match e.status.as_deref().map(|s| s.trim().to_lowercase()).as_deref() {
            None => {}
            Some("todo") => patch.status = Some(TaskStatus::Todo),
            Some("doing") => patch.status = Some(TaskStatus::Doing),
            Some("done") => complete = true,
            Some(o) => return Err(invalid(format!("status \"{o}\": use todo, doing or done"))),
        }
        let mut t = if patch.title.is_some() || patch.notes.is_some() || patch.priority.is_some() || patch.due_at.is_some() || patch.remind_at.is_some() || patch.recurrence.is_some() || patch.planner_id.is_some() || patch.status.is_some() {
            self.update_task(t.id, patch)?
        } else {
            t
        };
        if complete {
            t = self.complete_task(t.id)?;
        }
        Ok(format!("Updated [{}] {}{}.", handle(t.id), t.title, if t.status == TaskStatus::Done { " (done)" } else { "" }))
    }

    pub fn agent_task_delete(&self, which: &str) -> Result<String> {
        let t = self.find_task(which)?;
        self.delete_task(t.id)?;
        Ok(format!("Deleted the task {}.", t.title))
    }

    // ---- calendar events ----

    fn event_times(start: &str, end: Option<&str>) -> Result<(Timestamp, Timestamp, bool)> {
        let (s, timed) = parse_when(start)?;
        let end = end.map(str::trim).filter(|e| !e.is_empty());
        let e = match end {
            Some(e) => {
                let (e, e_timed) = parse_when(e)?;
                if timed || e_timed { e } else { end_of_day(e) }
            }
            None if timed => s + Duration::hours(1).num_milliseconds(),
            None => end_of_day(s),
        };
        let (s, e) = if timed { (s, e) } else { (s, e.max(end_of_day(s))) };
        if e < s {
            return Err(invalid("the event ends before it starts"));
        }
        Ok((s, e, !timed))
    }

    pub fn agent_event_add(&self, ev: &AgentEvent) -> Result<String> {
        let (start_at, end_at, all_day) = Self::event_times(&ev.start, ev.end.as_deref())?;
        let remind_at = ev.remind_minutes.filter(|m| *m >= 0).map(|m| start_at - m * 60_000);
        let e = self.create_event(EventInput {
            title: ev.title.clone(),
            description: ev.description.clone().unwrap_or_default(),
            location: ev.location.clone().unwrap_or_default(),
            start_at,
            end_at,
            all_day,
            remind_at,
            color: None,
        })?;
        Ok(format!("Added [{}] {} on {}.", handle(e.id), e.title, show(e.start_at, e.all_day)))
    }

    pub fn agent_event_edit(&self, which: &str, ed: &AgentEventEdit) -> Result<String> {
        let e = self.find_event(which)?;
        let (mut start_at, mut end_at, mut all_day) = (e.start_at, e.end_at, e.all_day);
        if ed.start.is_some() || ed.end.is_some() {
            let start = match &ed.start {
                Some(s) => s.clone(),
                None => if all_day { at(start_at).format("%Y-%m-%d").to_string() } else { at(start_at).format("%Y-%m-%d %H:%M").to_string() },
            };
            // Moving only the start keeps the event's length.
            let end = match (&ed.end, &ed.start) {
                (Some(e), _) => Some(e.clone()),
                (None, Some(_)) if !all_day => {
                    let (s, _) = parse_when(&start)?;
                    Some(at(s + (end_at - start_at)).format("%Y-%m-%d %H:%M").to_string())
                }
                _ => None,
            };
            (start_at, end_at, all_day) = Self::event_times(&start, end.as_deref())?;
        }
        let remind_at = match ed.remind_minutes {
            Some(m) if m >= 0 => Some(start_at - m * 60_000),
            Some(_) => None,
            None => e.remind_at,
        };
        let e = self.update_event(
            e.id,
            EventInput {
                title: ed.title.clone().unwrap_or(e.title),
                description: ed.description.clone().unwrap_or(e.description),
                location: ed.location.clone().unwrap_or(e.location),
                start_at,
                end_at,
                all_day,
                remind_at,
                color: e.color,
            },
        )?;
        Ok(format!("Updated [{}] {} — {}.", handle(e.id), e.title, show(e.start_at, e.all_day)))
    }

    pub fn agent_event_delete(&self, which: &str) -> Result<String> {
        let e = self.find_event(which)?;
        self.delete_event(e.id)?;
        Ok(format!("Removed {} from the calendar.", e.title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_plan_an_app_and_fill_the_calendar() {
        let s = Store::open_in_memory().unwrap();
        let task = |title: &str, due: Option<&str>, planner: Option<&str>| AgentTask { title: title.into(), due: due.map(Into::into), planner: planner.map(Into::into), ..Default::default() };
        let msg = s
            .agent_planner_create("Todo app", "Ship v1", &[task("Design the schema", Some("2030-05-01"), None), AgentTask { priority: Some("high".into()), ..task("Build the API", Some("2030-05-03 17:00"), None) }])
            .unwrap();
        assert!(msg.contains("2 tasks"), "{msg}");
        // A bad task means no planner is made.
        assert!(s.agent_planner_create("Broken", "", &[AgentTask { priority: Some("huge".into()), ..task("x", None, None) }]).is_err());
        assert_eq!(s.list_planners().unwrap().len(), 1);

        s.agent_tasks_add(&[task("Write tests", None, Some("todo app"))]).unwrap();
        let list = s.agent_tasks(Some("Todo app"), false, None, 50).unwrap();
        assert!(list.contains("Design the schema") && list.contains("due Wed 2030-05-01") && list.contains("high") && list.contains("Write tests"), "{list}");

        // Edit by title, finish one, move a deadline.
        s.agent_task_edit("Design the schema", &AgentTaskEdit { status: Some("done".into()), ..Default::default() }).unwrap();
        s.agent_task_edit("Build the API", &AgentTaskEdit { due: Some("2030-05-10 09:00".into()), ..Default::default() }).unwrap();
        let list = s.agent_tasks(Some("Todo app"), false, None, 50).unwrap();
        assert!(!list.contains("Design the schema") && list.contains("2030-05-10 09:00"), "{list}");
        assert_eq!(s.list_planners().unwrap()[0].done_count, 1);

        // Repeating task: done moves it on instead of closing it.
        s.agent_tasks_add(&[AgentTask { repeat: Some("weekly".into()), ..task("Backup", Some("2030-06-03"), None) }]).unwrap();
        s.agent_task_edit("Backup", &AgentTaskEdit { status: Some("done".into()), ..Default::default() }).unwrap();
        assert!(s.agent_tasks(None, false, None, 50).unwrap().contains("due Mon 2030-06-10"));

        // Calendar: timed, all-day, edit, delete.
        s.agent_event_add(&AgentEvent { title: "Dentist".into(), start: "2030-05-02 14:30".into(), location: Some("Main St".into()), remind_minutes: Some(30), ..Default::default() }).unwrap();
        s.agent_event_add(&AgentEvent { title: "Holiday".into(), start: "2030-05-06".into(), end: Some("2030-05-08".into()), ..Default::default() }).unwrap();
        let cal = s.agent_calendar("2030-05-01", "2030-05-09").unwrap();
        assert!(cal.contains("Dentist") && cal.contains("14:30–15:30") && cal.contains("@ Main St") && cal.contains("Holiday") && cal.contains("task"), "{cal}");
        s.agent_event_edit("Dentist", &AgentEventEdit { start: Some("2030-05-02 16:00".into()), ..Default::default() }).unwrap();
        assert!(s.agent_calendar("2030-05-02", "2030-05-02").unwrap().contains("16:00–17:00"));
        s.agent_event_delete("Dentist").unwrap();
        assert!(!s.agent_calendar("2030-05-01", "2030-05-09").unwrap().contains("Dentist"));

        // Deleting a planner keeps its tasks unless asked.
        s.agent_planner_delete("Todo app", false).unwrap();
        assert!(s.agent_tasks(None, false, None, 50).unwrap().contains("Write tests"));
        assert!(parse_when("tomorrow").is_err());
    }
}
