//! Planners group tasks into projects (shown as boards: to do / doing / done).

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::util::{Timestamp, now_ms, uuid_col};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Planner {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub color: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub task_count: i64,
    pub done_count: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlannerInput {
    pub name: String,
    pub description: String,
    pub color: Option<String>,
}

const SELECT: &str = "SELECT p.id, p.name, p.description, p.color, p.created_at, p.updated_at,
    (SELECT COUNT(*) FROM tasks t WHERE t.planner_id = p.id),
    (SELECT COUNT(*) FROM tasks t WHERE t.planner_id = p.id AND t.status = 'done')
    FROM planners p";

fn row_to_planner(r: &Row<'_>) -> rusqlite::Result<Planner> {
    Ok(Planner {
        id: uuid_col(r, 0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        color: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
        task_count: r.get(6)?,
        done_count: r.get(7)?,
    })
}

impl Store {
    pub fn create_planner(&self, input: PlannerInput) -> Result<Planner> {
        if input.name.trim().is_empty() {
            return Err(Error::invalid("planner needs a name"));
        }
        let id = Uuid::now_v7();
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO planners(id, name, description, color, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                id.as_bytes(),
                input.name.trim(),
                input.description,
                input.color.unwrap_or_else(|| "#7c6cff".into()),
                now
            ],
        )?;
        self.get_planner(id)
    }

    pub fn get_planner(&self, id: Uuid) -> Result<Planner> {
        self.conn
            .query_row(
                &format!("{SELECT} WHERE p.id = ?1"),
                [id.as_bytes()],
                row_to_planner,
            )
            .optional()?
            .ok_or(Error::NotFound("planner", id))
    }

    pub fn list_planners(&self) -> Result<Vec<Planner>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{SELECT} ORDER BY p.created_at"))?;
        let rows = stmt.query_map([], row_to_planner)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn update_planner(&self, id: Uuid, input: PlannerInput) -> Result<Planner> {
        if input.name.trim().is_empty() {
            return Err(Error::invalid("planner needs a name"));
        }
        let n = self.conn.execute(
            "UPDATE planners SET name = ?2, description = ?3, color = COALESCE(?4, color), updated_at = ?5
             WHERE id = ?1",
            params![id.as_bytes(), input.name.trim(), input.description, input.color, now_ms()],
        )?;
        if n == 0 {
            return Err(Error::NotFound("planner", id));
        }
        self.get_planner(id)
    }

    /// Deletes the planner. Its tasks are kept, just no longer assigned to a planner.
    pub fn delete_planner(&self, id: Uuid) -> Result<()> {
        if self
            .conn
            .execute("DELETE FROM planners WHERE id = ?1", [id.as_bytes()])?
            == 0
        {
            return Err(Error::NotFound("planner", id));
        }
        Ok(())
    }
}
