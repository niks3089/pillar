use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use prost::Message;
use rusqlite::{params, Connection};
use serde::Serialize;

use pillar_shared::proto::{LogEntry, NodeStatus, RegisterNodeRequest};

pub type Db = Arc<Mutex<Connection>>;

pub fn open_db(path: &str) -> Result<Db> {
    let conn = Connection::open(path).context("opening SQLite database")?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .context("enabling WAL mode")?;
    init_schema(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS nodes (
            node_id TEXT PRIMARY KEY,
            lifecycle_state TEXT NOT NULL DEFAULT 'registered',
            role TEXT,
            client TEXT,
            cluster TEXT,
            hostname TEXT,
            architecture TEXT,
            os TEXT,
            agent_version TEXT,
            ip_address TEXT,
            last_seen_at INTEGER,
            registered_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS status_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id TEXT NOT NULL,
            status_blob BLOB NOT NULL,
            received_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_status_history_node_time
            ON status_history(node_id, received_at);

        CREATE TABLE IF NOT EXISTS logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            node_id TEXT NOT NULL,
            service TEXT NOT NULL,
            level TEXT NOT NULL,
            message TEXT NOT NULL,
            unit TEXT,
            timestamp_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_logs_node_time
            ON logs(node_id, timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_logs_node_service
            ON logs(node_id, service, timestamp_ms);

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )
    .context("initializing schema")?;

    // Migration: add ip_address column if missing (safe for existing DBs)
    let _ = conn.execute_batch("ALTER TABLE nodes ADD COLUMN ip_address TEXT;");

    // Migration: add provision_config_json column if missing
    let _ = conn.execute_batch("ALTER TABLE nodes ADD COLUMN provision_config_json TEXT;");

    // Migration: merge operator_version + link_version → agent_version
    let _ = conn.execute_batch("ALTER TABLE nodes ADD COLUMN agent_version TEXT;");

    Ok(())
}

fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct NodeRow {
    pub node_id: String,
    pub lifecycle_state: String,
    pub role: Option<String>,
    pub client: Option<String>,
    pub cluster: Option<String>,
    pub hostname: Option<String>,
    pub agent_version: Option<String>,
    pub ip_address: Option<String>,
    pub last_seen_at: Option<i64>,
    pub registered_at: Option<i64>,
    /// Populated at runtime from the in-memory NodeRegistry, not from SQLite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_status: Option<NodeStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusHistoryRow {
    pub id: i64,
    pub node_id: String,
    pub status: NodeStatus,
    pub received_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogRow {
    pub id: i64,
    pub node_id: String,
    pub service: String,
    pub level: String,
    pub message: String,
    pub unit: Option<String>,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetOverview {
    pub total: usize,
    pub by_state: HashMap<String, usize>,
    /// Populated at runtime from the in-memory NodeRegistry, not from SQLite.
    #[serde(default)]
    pub connected_nodes: u32,
}

// ---------------------------------------------------------------------------
// CRUD operations (async wrappers around blocking SQLite calls)
// ---------------------------------------------------------------------------

pub async fn upsert_node(db: &Db, req: &RegisterNodeRequest, ip_address: &str) -> Result<()> {
    let db = db.clone();
    let req = req.clone();
    let ip_address = ip_address.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let now = now_epoch_secs();
        conn.execute(
            "INSERT OR REPLACE INTO nodes
                (node_id, role, client, cluster, hostname, architecture, os,
                 agent_version, ip_address, registered_at, lifecycle_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'registered')",
            params![
                req.node_id,
                optional_str(&req.role),
                optional_str(&req.client),
                optional_str(&req.cluster),
                optional_str(&req.hostname),
                optional_str(&req.architecture),
                optional_str(&req.os),
                optional_str(&req.agent_version),
                optional_str(&ip_address),
                now,
            ],
        )
        .context("upsert_node")?;
        Ok(())
    })
    .await?
}

pub async fn update_node_status(db: &Db, node_id: &str, status: &NodeStatus) -> Result<()> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    let status = status.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let now = now_epoch_secs();

        // Map operator state string to lifecycle state
        let lifecycle = map_state_to_lifecycle(&status.state);

        conn.execute(
            "UPDATE nodes SET lifecycle_state = ?1, last_seen_at = ?2 WHERE node_id = ?3",
            params![lifecycle, now, node_id],
        )
        .context("update lifecycle_state")?;

        let status_blob = status.encode_to_vec();
        conn.execute(
            "INSERT INTO status_history (node_id, status_blob, received_at) VALUES (?1, ?2, ?3)",
            params![node_id, status_blob, now],
        )
        .context("insert status_history")?;

        Ok(())
    })
    .await?
}

pub async fn get_node(db: &Db, node_id: &str) -> Result<Option<NodeRow>> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let sql = format!("SELECT {NODE_SELECT_COLUMNS} FROM nodes WHERE node_id = ?1");
        let mut stmt = conn.prepare(&sql).context("prepare get_node")?;

        let row = stmt
            .query_row(params![node_id], row_to_node)
            .optional()
            .context("query get_node")?;

        Ok(row)
    })
    .await?
}

pub async fn list_nodes(db: &Db) -> Result<Vec<NodeRow>> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let sql = format!("SELECT {NODE_SELECT_COLUMNS} FROM nodes ORDER BY registered_at DESC");
        let mut stmt = conn.prepare(&sql).context("prepare list_nodes")?;

        let rows = stmt
            .query_map([], row_to_node)
            .context("query list_nodes")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("collect list_nodes")?;

        Ok(rows)
    })
    .await?
}

pub async fn delete_node(db: &Db, node_id: &str) -> Result<bool> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let deleted = conn
            .execute("DELETE FROM nodes WHERE node_id = ?1", params![node_id])
            .context("delete_node")?;
        Ok(deleted > 0)
    })
    .await?
}

pub async fn get_lifecycle_state(db: &Db, node_id: &str) -> Result<Option<String>> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let state: Option<String> = conn
            .query_row(
                "SELECT lifecycle_state FROM nodes WHERE node_id = ?1",
                params![node_id],
                |row| row.get(0),
            )
            .optional()
            .context("get_lifecycle_state")?;
        Ok(state)
    })
    .await?
}

pub async fn set_lifecycle_state(db: &Db, node_id: &str, state: &str) -> Result<()> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    let state = state.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE nodes SET lifecycle_state = ?1 WHERE node_id = ?2",
            params![state, node_id],
        )
        .context("set_lifecycle_state")?;
        Ok(())
    })
    .await?
}

pub async fn set_provision_config(db: &Db, node_id: &str, config_json: &str) -> Result<()> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    let config_json = config_json.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "UPDATE nodes SET provision_config_json = ?1 WHERE node_id = ?2",
            params![config_json, node_id],
        )
        .context("set_provision_config")?;
        Ok(())
    })
    .await?
}

pub async fn get_status_history(
    db: &Db,
    node_id: &str,
    limit: u32,
) -> Result<Vec<StatusHistoryRow>> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let limit_i64 = limit as i64;
        let mut stmt = conn
            .prepare(
                "SELECT id, node_id, status_blob, received_at
                 FROM status_history
                 WHERE node_id = ?1
                 ORDER BY received_at DESC
                 LIMIT ?2",
            )
            .context("prepare get_status_history")?;

        let rows = stmt
            .query_map(params![node_id, limit_i64], |row| {
                let id: i64 = row.get(0)?;
                let node_id: String = row.get(1)?;
                let blob: Vec<u8> = row.get(2)?;
                let received_at: i64 = row.get(3)?;
                let status = NodeStatus::decode(blob.as_slice())
                    .unwrap_or_default();
                Ok(StatusHistoryRow {
                    id,
                    node_id,
                    status,
                    received_at,
                })
            })
            .context("query get_status_history")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("collect get_status_history")?;

        Ok(rows)
    })
    .await?
}

pub async fn insert_logs(db: &Db, node_id: &str, entries: &[LogEntry]) -> Result<u64> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    let entries = entries.to_vec();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut count = 0u64;
        let mut stmt = conn
            .prepare(
                "INSERT INTO logs (node_id, service, level, message, unit, timestamp_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .context("prepare insert_logs")?;

        for entry in &entries {
            stmt.execute(params![
                node_id,
                entry.service,
                entry.level,
                entry.message,
                optional_str(&entry.unit),
                entry.timestamp_unix_ms,
            ])
            .context("insert log entry")?;
            count += 1;
        }

        Ok(count)
    })
    .await?
}

pub async fn get_logs(
    db: &Db,
    node_id: &str,
    service: Option<&str>,
    level: Option<&str>,
    since_ms: Option<i64>,
    limit: u32,
) -> Result<Vec<LogRow>> {
    let db = db.clone();
    let node_id = node_id.to_owned();
    let service = service.map(|s| s.to_owned());
    let level = level.map(|l| l.to_owned());
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        let mut sql = String::from(
            "SELECT id, node_id, service, level, message, unit, timestamp_ms
             FROM logs WHERE node_id = ?1",
        );
        let mut param_idx = 2u32;
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(node_id.clone())];

        if let Some(ref svc) = service {
            sql.push_str(&format!(" AND service = ?{param_idx}"));
            param_idx += 1;
            param_values.push(Box::new(svc.clone()));
        }
        if let Some(ref lvl) = level {
            sql.push_str(&format!(" AND level = ?{param_idx}"));
            param_idx += 1;
            param_values.push(Box::new(lvl.clone()));
        }
        if let Some(since) = since_ms {
            sql.push_str(&format!(" AND timestamp_ms >= ?{param_idx}"));
            param_idx += 1;
            param_values.push(Box::new(since));
        }

        sql.push_str(&format!(" ORDER BY timestamp_ms DESC LIMIT ?{param_idx}"));
        param_values.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&sql).context("prepare get_logs")?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok(LogRow {
                    id: row.get(0)?,
                    node_id: row.get(1)?,
                    service: row.get(2)?,
                    level: row.get(3)?,
                    message: row.get(4)?,
                    unit: row.get(5)?,
                    timestamp_ms: row.get(6)?,
                })
            })
            .context("query get_logs")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("collect get_logs")?;

        Ok(rows)
    })
    .await?
}

pub async fn get_fleet_overview(db: &Db) -> Result<FleetOverview> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT lifecycle_state, COUNT(*) FROM nodes GROUP BY lifecycle_state")
            .context("prepare get_fleet_overview")?;

        let mut by_state = HashMap::new();
        let mut total = 0usize;

        let rows = stmt
            .query_map([], |row| {
                let state: String = row.get(0)?;
                let count: usize = row.get(1)?;
                Ok((state, count))
            })
            .context("query get_fleet_overview")?;

        for row in rows {
            let (state, count) = row.context("read fleet overview row")?;
            total += count;
            by_state.insert(state, count);
        }

        Ok(FleetOverview {
            total,
            by_state,
            connected_nodes: 0,
        })
    })
    .await?
}

pub async fn get_setting(db: &Db, key: &str) -> Result<Option<String>> {
    let db = db.clone();
    let key = key.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .context("get_setting")?;
        Ok(value)
    })
    .await?
}

pub async fn set_setting(db: &Db, key: &str, value: &str) -> Result<()> {
    let db = db.clone();
    let key = key.to_owned();
    let value = value.to_owned();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .context("set_setting")?;
        Ok(())
    })
    .await?
}

pub async fn prune_old_data(db: &Db, retention_days: u32) -> Result<(usize, usize)> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let cutoff = now_epoch_secs() - (retention_days as i64 * 86400);

        let status_deleted = conn
            .execute(
                "DELETE FROM status_history WHERE received_at < ?1",
                params![cutoff],
            )
            .context("prune status_history")?;

        let cutoff_ms = cutoff * 1000;
        let logs_deleted = conn
            .execute(
                "DELETE FROM logs WHERE timestamp_ms < ?1",
                params![cutoff_ms],
            )
            .context("prune logs")?;

        Ok((status_deleted, logs_deleted))
    })
    .await?
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert empty proto strings to None for nullable DB columns.
fn optional_str(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Map operator state strings to lifecycle states for the UI.
fn map_state_to_lifecycle(state: &str) -> &str {
    match state {
        "off" => "offline",
        "starting_up" => "starting_up",
        "behind" => "behind",
        "healthy" => "healthy",
        "recovering" => "recovering",
        _ => state,
    }
}

// We need the optional() method on query_row results.
use rusqlite::OptionalExtension;

const NODE_SELECT_COLUMNS: &str =
    "node_id, lifecycle_state, role, client, cluster, hostname,
     agent_version, ip_address, last_seen_at, registered_at";

fn row_to_node(row: &rusqlite::Row) -> rusqlite::Result<NodeRow> {
    Ok(NodeRow {
        node_id: row.get(0)?,
        lifecycle_state: row.get(1)?,
        role: row.get(2)?,
        client: row.get(3)?,
        cluster: row.get(4)?,
        hostname: row.get(5)?,
        agent_version: row.get(6)?,
        ip_address: row.get(7)?,
        last_seen_at: row.get(8)?,
        registered_at: row.get(9)?,
        live_status: None,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        init_schema(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    fn sample_register_request() -> RegisterNodeRequest {
        RegisterNodeRequest {
            node_id: "node-1".to_string(),
            role: "rpc".to_string(),
            client: "agave".to_string(),
            cluster: "mainnet".to_string(),
            hostname: "host-1".to_string(),
            architecture: "x86_64".to_string(),
            os: "linux".to_string(),
            agent_version: "0.1.0".to_string(),
        }
    }

    fn sample_status() -> NodeStatus {
        NodeStatus {
            state: "healthy".to_string(),
            local_slot: 1000,
            reference_slot: 1005,
            slots_behind: 5,
            healthy: true,
            role: "rpc".to_string(),
            client: "agave".to_string(),
            cluster: "mainnet".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn upsert_and_get_node() {
        let db = test_db();
        let req = sample_register_request();
        upsert_node(&db, &req, "10.0.0.1").await.unwrap();

        let node = get_node(&db, "node-1").await.unwrap().unwrap();
        assert_eq!(node.node_id, "node-1");
        assert_eq!(node.lifecycle_state, "registered");
        assert_eq!(node.role.as_deref(), Some("rpc"));
        assert_eq!(node.client.as_deref(), Some("agave"));
        assert!(node.registered_at.is_some());
    }

    #[tokio::test]
    async fn get_nonexistent_node() {
        let db = test_db();
        let node = get_node(&db, "nonexistent").await.unwrap();
        assert!(node.is_none());
    }

    #[tokio::test]
    async fn list_nodes_empty() {
        let db = test_db();
        let nodes = list_nodes(&db).await.unwrap();
        assert!(nodes.is_empty());
    }

    #[tokio::test]
    async fn list_nodes_with_data() {
        let db = test_db();
        upsert_node(&db, &sample_register_request(), "10.0.0.1").await.unwrap();
        let mut req2 = sample_register_request();
        req2.node_id = "node-2".to_string();
        upsert_node(&db, &req2, "10.0.0.2").await.unwrap();

        let nodes = list_nodes(&db).await.unwrap();
        assert_eq!(nodes.len(), 2);
    }

    #[tokio::test]
    async fn update_status_and_history() {
        let db = test_db();
        upsert_node(&db, &sample_register_request(), "10.0.0.1").await.unwrap();

        let status = sample_status();
        update_node_status(&db, "node-1", &status).await.unwrap();

        let node = get_node(&db, "node-1").await.unwrap().unwrap();
        assert_eq!(node.lifecycle_state, "healthy");
        assert!(node.last_seen_at.is_some());

        let history = get_status_history(&db, "node-1", 10u32).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status.state, "healthy");
    }

    #[tokio::test]
    async fn delete_node_found() {
        let db = test_db();
        upsert_node(&db, &sample_register_request(), "10.0.0.1").await.unwrap();
        assert!(delete_node(&db, "node-1").await.unwrap());
        assert!(get_node(&db, "node-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_node_not_found() {
        let db = test_db();
        assert!(!delete_node(&db, "nonexistent").await.unwrap());
    }

    #[tokio::test]
    async fn insert_and_get_logs() {
        let db = test_db();
        let entries = vec![
            LogEntry {
                service: "validator".to_string(),
                timestamp_unix_ms: 1000,
                level: "info".to_string(),
                message: "started".to_string(),
                unit: "solana-validator.service".to_string(),
            },
            LogEntry {
                service: "operator".to_string(),
                timestamp_unix_ms: 2000,
                level: "error".to_string(),
                message: "failed".to_string(),
                unit: String::new(),
            },
        ];

        let count = insert_logs(&db, "node-1", &entries).await.unwrap();
        assert_eq!(count, 2);

        let logs = get_logs(&db, "node-1", None, None, None, 100u32).await.unwrap();
        assert_eq!(logs.len(), 2);

        // Filter by service
        let logs = get_logs(&db, "node-1", Some("validator"), None, None, 100u32)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "started");

        // Filter by level
        let logs = get_logs(&db, "node-1", None, Some("error"), None, 100u32)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].service, "operator");

        // Filter by since_ms
        let logs = get_logs(&db, "node-1", None, None, Some(1500), 100u32)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
    }

    #[tokio::test]
    async fn fleet_overview() {
        let db = test_db();
        upsert_node(&db, &sample_register_request(), "10.0.0.1").await.unwrap();

        let mut req2 = sample_register_request();
        req2.node_id = "node-2".to_string();
        upsert_node(&db, &req2, "10.0.0.2").await.unwrap();

        // Move node-1 to healthy
        update_node_status(&db, "node-1", &sample_status())
            .await
            .unwrap();

        let overview = get_fleet_overview(&db).await.unwrap();
        assert_eq!(overview.total, 2);
        assert_eq!(overview.by_state.get("healthy"), Some(&1));
        assert_eq!(overview.by_state.get("registered"), Some(&1));
    }

    #[tokio::test]
    async fn get_set_setting() {
        let db = test_db();

        // Missing key returns None
        assert!(get_setting(&db, "grafana_url").await.unwrap().is_none());

        // Set and get
        set_setting(&db, "grafana_url", "https://grafana.example.com")
            .await
            .unwrap();
        assert_eq!(
            get_setting(&db, "grafana_url").await.unwrap().as_deref(),
            Some("https://grafana.example.com")
        );

        // Overwrite
        set_setting(&db, "grafana_url", "https://new.example.com")
            .await
            .unwrap();
        assert_eq!(
            get_setting(&db, "grafana_url").await.unwrap().as_deref(),
            Some("https://new.example.com")
        );
    }

    #[tokio::test]
    async fn prune_removes_old_data() {
        let db = test_db();
        // Insert old status_history directly
        {
            let conn = db.lock().unwrap();
            let old_time = now_epoch_secs() - 100 * 86400; // 100 days ago
            let empty_status = NodeStatus::default().encode_to_vec();
            conn.execute(
                "INSERT INTO status_history (node_id, status_blob, received_at) VALUES (?1, ?2, ?3)",
                params!["node-old", empty_status, old_time],
            )
            .unwrap();
            let old_time_ms = old_time * 1000;
            conn.execute(
                "INSERT INTO logs (node_id, service, level, message, timestamp_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["node-old", "validator", "info", "old log", old_time_ms],
            )
            .unwrap();
        }

        let (status_pruned, logs_pruned) = prune_old_data(&db, 30).await.unwrap();
        assert_eq!(status_pruned, 1);
        assert_eq!(logs_pruned, 1);
    }
}
