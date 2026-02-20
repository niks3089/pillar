use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::Db;

pub fn init_alert_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS alert_rules (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            field TEXT NOT NULL,
            operator TEXT NOT NULL,
            threshold TEXT NOT NULL,
            node_id_filter TEXT,
            enabled BOOLEAN NOT NULL DEFAULT TRUE,
            severity TEXT NOT NULL DEFAULT 'warning',
            cooldown_secs INTEGER NOT NULL DEFAULT 0,
            is_default BOOLEAN NOT NULL DEFAULT FALSE,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS alert_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id TEXT NOT NULL,
            rule_id TEXT NOT NULL,
            rule_name TEXT NOT NULL,
            severity TEXT NOT NULL,
            fired_at INTEGER NOT NULL,
            resolved_at INTEGER,
            trigger_value TEXT NOT NULL,
            notification_sent BOOLEAN NOT NULL DEFAULT FALSE
        );
        CREATE INDEX IF NOT EXISTS idx_alert_history_node ON alert_history(node_id, fired_at);
        ",
    )
    .context("initializing alert schema")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRuleRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub field: String,
    pub operator: String,
    pub threshold: String,
    pub node_id_filter: Option<String>,
    pub enabled: bool,
    pub severity: String,
    pub cooldown_secs: i64,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertHistoryRow {
    pub id: i64,
    pub node_id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub severity: String,
    pub fired_at: i64,
    pub resolved_at: Option<i64>,
    pub trigger_value: String,
    pub notification_sent: bool,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

const RULE_COLS: &str =
    "id, name, description, field, operator, threshold, node_id_filter,
     enabled, severity, cooldown_secs, is_default, created_at, updated_at";

fn row_to_rule(row: &rusqlite::Row) -> rusqlite::Result<AlertRuleRow> {
    Ok(AlertRuleRow {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        field: row.get(3)?,
        operator: row.get(4)?,
        threshold: row.get(5)?,
        node_id_filter: row.get(6)?,
        enabled: row.get(7)?,
        severity: row.get(8)?,
        cooldown_secs: row.get(9)?,
        is_default: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

const HISTORY_COLS: &str =
    "id, node_id, rule_id, rule_name, severity, fired_at, resolved_at,
     trigger_value, notification_sent";

fn row_to_history(row: &rusqlite::Row) -> rusqlite::Result<AlertHistoryRow> {
    Ok(AlertHistoryRow {
        id: row.get(0)?,
        node_id: row.get(1)?,
        rule_id: row.get(2)?,
        rule_name: row.get(3)?,
        severity: row.get(4)?,
        fired_at: row.get(5)?,
        resolved_at: row.get(6)?,
        trigger_value: row.get(7)?,
        notification_sent: row.get(8)?,
    })
}

// ---------------------------------------------------------------------------
// Rules CRUD
// ---------------------------------------------------------------------------

pub async fn list_rules(db: &Db) -> Result<Vec<AlertRuleRow>> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let sql = format!("SELECT {RULE_COLS} FROM alert_rules ORDER BY created_at");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_rule)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("collect alert_rules")?;
        Ok(rows)
    })
    .await?
}

pub async fn get_rule(db: &Db, id: &str) -> Result<Option<AlertRuleRow>> {
    let db = db.clone();
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        use rusqlite::OptionalExtension;
        let sql = format!("SELECT {RULE_COLS} FROM alert_rules WHERE id = ?1");
        conn.query_row(&sql, params![id], row_to_rule).optional().map_err(Into::into)
    })
    .await?
}

pub async fn insert_rule(db: &Db, rule: &AlertRuleRow) -> Result<()> {
    let db = db.clone();
    let rule = rule.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT INTO alert_rules (id,name,description,field,operator,threshold,
             node_id_filter,enabled,severity,cooldown_secs,is_default,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![rule.id, rule.name, rule.description, rule.field, rule.operator,
                    rule.threshold, rule.node_id_filter, rule.enabled, rule.severity,
                    rule.cooldown_secs, rule.is_default, rule.created_at, rule.updated_at],
        )?;
        Ok(())
    })
    .await?
}

/// Seed a default rule (INSERT OR IGNORE).
pub fn insert_rule_if_absent(conn: &Connection, rule: &AlertRuleRow) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO alert_rules (id,name,description,field,operator,threshold,
         node_id_filter,enabled,severity,cooldown_secs,is_default,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![rule.id, rule.name, rule.description, rule.field, rule.operator,
                rule.threshold, rule.node_id_filter, rule.enabled, rule.severity,
                rule.cooldown_secs, rule.is_default, rule.created_at, rule.updated_at],
    )?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct UpdateRuleRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub field: Option<String>,
    pub operator: Option<String>,
    pub threshold: Option<String>,
    pub node_id_filter: Option<String>,
    pub enabled: Option<bool>,
    pub severity: Option<String>,
    pub cooldown_secs: Option<i64>,
}

pub async fn update_rule(db: &Db, id: &str, req: &UpdateRuleRequest) -> Result<bool> {
    let db = db.clone();
    let id = id.to_owned();
    let req_name = req.name.clone();
    let req_desc = req.description.clone();
    let req_field = req.field.clone();
    let req_op = req.operator.clone();
    let req_threshold = req.threshold.clone();
    let req_filter = req.node_id_filter.clone();
    let req_enabled = req.enabled;
    let req_severity = req.severity.clone();
    let req_cooldown = req.cooldown_secs;
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let now = now_secs();
        let mut sets = vec!["updated_at = ?1".to_string()];
        let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
        let mut i = 2u32;

        macro_rules! maybe {
            ($opt:expr, $col:expr) => {
                if let Some(ref v) = $opt { sets.push(format!("{} = ?{i}", $col)); vals.push(Box::new(v.clone())); i += 1; }
            };
        }
        maybe!(req_name, "name");
        maybe!(req_desc, "description");
        maybe!(req_field, "field");
        maybe!(req_op, "operator");
        maybe!(req_threshold, "threshold");
        maybe!(req_filter, "node_id_filter");
        maybe!(req_severity, "severity");
        if let Some(v) = req_enabled { sets.push(format!("enabled = ?{i}")); vals.push(Box::new(v)); i += 1; }
        if let Some(v) = req_cooldown { sets.push(format!("cooldown_secs = ?{i}")); vals.push(Box::new(v)); i += 1; }

        let sql = format!("UPDATE alert_rules SET {} WHERE id = ?{i}", sets.join(", "));
        vals.push(Box::new(id));
        let refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|p| p.as_ref()).collect();
        Ok(conn.execute(&sql, refs.as_slice())? > 0)
    })
    .await?
}

pub async fn delete_rule(db: &Db, id: &str) -> Result<bool> {
    let db = db.clone();
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        Ok(conn.execute("DELETE FROM alert_rules WHERE id = ?1 AND is_default = FALSE", params![id])? > 0)
    })
    .await?
}

// ---------------------------------------------------------------------------
// History CRUD
// ---------------------------------------------------------------------------

pub async fn insert_history(db: &Db, node_id: &str, rule_id: &str, rule_name: &str,
                            severity: &str, trigger_value: &str) -> Result<i64> {
    let db = db.clone();
    let (nid, rid, rn, sev, tv) = (node_id.to_owned(), rule_id.to_owned(),
        rule_name.to_owned(), severity.to_owned(), trigger_value.to_owned());
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT INTO alert_history (node_id,rule_id,rule_name,severity,fired_at,trigger_value)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![nid, rid, rn, sev, now_secs(), tv],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .await?
}

pub async fn resolve_history(db: &Db, node_id: &str, rule_id: &str) -> Result<()> {
    let db = db.clone();
    let (nid, rid) = (node_id.to_owned(), rule_id.to_owned());
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE alert_history SET resolved_at = ?1 WHERE node_id = ?2 AND rule_id = ?3 AND resolved_at IS NULL",
            params![now_secs(), nid, rid],
        )?;
        Ok(())
    })
    .await?
}

pub async fn mark_notified(db: &Db, history_id: i64) -> Result<()> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute("UPDATE alert_history SET notification_sent = TRUE WHERE id = ?1", params![history_id])?;
        Ok(())
    })
    .await?
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub node_id: Option<String>,
    pub severity: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}
fn default_limit() -> u32 { 100 }

pub async fn list_history(db: &Db, query: &HistoryQuery) -> Result<Vec<AlertHistoryRow>> {
    let db = db.clone();
    let (nid, sev, limit) = (query.node_id.clone(), query.severity.clone(), query.limit);
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut sql = format!("SELECT {HISTORY_COLS} FROM alert_history WHERE 1=1");
        let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
        let mut i = 1u32;
        if let Some(ref n) = nid { sql.push_str(&format!(" AND node_id = ?{i}")); vals.push(Box::new(n.clone())); i += 1; }
        if let Some(ref s) = sev { sql.push_str(&format!(" AND severity = ?{i}")); vals.push(Box::new(s.clone())); i += 1; }
        sql.push_str(&format!(" ORDER BY fired_at DESC LIMIT ?{i}"));
        vals.push(Box::new(limit as i64));
        let refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), row_to_history)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await?
}

pub async fn list_active(db: &Db) -> Result<Vec<AlertHistoryRow>> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let sql = format!("SELECT {HISTORY_COLS} FROM alert_history WHERE resolved_at IS NULL ORDER BY fired_at DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_history)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await?
}
