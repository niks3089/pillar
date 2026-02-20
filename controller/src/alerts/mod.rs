pub mod api;
pub mod db;
pub mod defaults;
pub mod notify;
pub mod rules;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::RwLock;

use pillar_shared::proto::NodeStatus;

use crate::db::Db;
use db::AlertRuleRow;
use notify::{AlertNotification, SendGridConfig};
use rules::{evaluate_condition, extract_field, rule_applies_to_node, Operator};

/// Minimal in-memory firing state: (node_id, rule_id) → (firing, last_transition_epoch)
type FiringMap = HashMap<(String, String), (bool, i64)>;

#[derive(Clone)]
pub struct AlertEngine {
    inner: Arc<RwLock<Inner>>,
    database: Db,
}

struct Inner {
    rules: Vec<AlertRuleRow>,
    sendgrid: SendGridConfig,
    firing: FiringMap,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl AlertEngine {
    pub fn new(database: Db) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                rules: Vec::new(),
                sendgrid: SendGridConfig::default(),
                firing: HashMap::new(),
            })),
            database,
        }
    }

    pub async fn init(&self) -> Result<()> {
        {
            let conn = self.database.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            defaults::seed(&conn)?;
        }
        self.reload().await
    }

    pub async fn reload(&self) -> Result<()> {
        let rules = db::list_rules(&self.database).await?;
        let sendgrid = notify::load_config(&self.database).await;
        let mut inner = self.inner.write().await;
        inner.rules = rules;
        inner.sendgrid = sendgrid;
        Ok(())
    }

    pub async fn evaluate(&self, node_id: &str, status: &NodeStatus) {
        let (rules, sendgrid) = {
            let inner = self.inner.read().await;
            (inner.rules.clone(), inner.sendgrid.clone())
        };

        for rule in &rules {
            if !rule.enabled || !rule_applies_to_node(rule, node_id) {
                continue;
            }
            let Some(op) = Operator::parse(&rule.operator) else { continue };
            let Some(value) = extract_field(status, &rule.field) else { continue };

            let now_firing = evaluate_condition(&value, op, &rule.threshold);
            let value_str = value.to_string();
            let key = (node_id.to_string(), rule.id.clone());

            let (was_firing, last_at) = {
                let inner = self.inner.read().await;
                inner.firing.get(&key).copied().unwrap_or((false, 0))
            };

            if now_firing == was_firing {
                continue;
            }

            // Cooldown check
            if now_firing && rule.cooldown_secs > 0 && now_secs() - last_at < rule.cooldown_secs {
                continue;
            }

            // Update in-memory state
            {
                let mut inner = self.inner.write().await;
                inner.firing.insert(key, (now_firing, now_secs()));
            }

            let db = self.database.clone();
            let nid = node_id.to_string();
            let rule = rule.clone();
            let sg = sendgrid.clone();

            if now_firing {
                tracing::info!(node_id, rule = %rule.name, severity = %rule.severity, value = %value_str, "alert FIRING");
                tokio::spawn(async move {
                    let hid = db::insert_history(&db, &nid, &rule.id, &rule.name, &rule.severity, &value_str).await;
                    if sg.is_configured() {
                        let alert = AlertNotification {
                            node_id: nid, rule_name: rule.name, severity: rule.severity,
                            firing: true, value: value_str, threshold: rule.threshold, field: rule.field,
                        };
                        if let Err(e) = notify::send(&sg, &alert).await {
                            tracing::warn!(error = %e, "sendgrid notification failed");
                        } else if let Ok(id) = hid {
                            let _ = db::mark_notified(&db, id).await;
                        }
                    }
                });
            } else {
                tracing::info!(node_id, rule = %rule.name, "alert RESOLVED");
                tokio::spawn(async move {
                    let _ = db::resolve_history(&db, &nid, &rule.id).await;
                    if sg.is_configured() {
                        let alert = AlertNotification {
                            node_id: nid, rule_name: rule.name, severity: rule.severity,
                            firing: false, value: value_str, threshold: rule.threshold, field: rule.field,
                        };
                        if let Err(e) = notify::send(&sg, &alert).await {
                            tracing::warn!(error = %e, "sendgrid resolved notification failed");
                        }
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Mutex;

    fn test_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        crate::db::init_schema_for_test(&conn);
        db::init_alert_schema(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[tokio::test]
    async fn evaluate_fires_and_resolves() {
        let database = test_db();
        let engine = AlertEngine::new(database.clone());
        {
            let conn = database.lock().unwrap();
            db::insert_rule_if_absent(&conn, &AlertRuleRow {
                id: "test_offline".into(), name: "Test Offline".into(),
                description: String::new(), field: "state".into(),
                operator: "eq".into(), threshold: "off".into(),
                node_id_filter: None, enabled: true, severity: "critical".into(),
                cooldown_secs: 0, is_default: false, created_at: 0, updated_at: 0,
            }).unwrap();
        }
        engine.reload().await.unwrap();

        let healthy = NodeStatus { state: "healthy".into(), ..Default::default() };
        engine.evaluate("node-1", &healthy).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert!(db::list_active(&database).await.unwrap().is_empty());

        let off = NodeStatus { state: "off".into(), ..Default::default() };
        engine.evaluate("node-1", &off).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let active = db::list_active(&database).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].rule_id, "test_offline");

        engine.evaluate("node-1", &healthy).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        assert!(db::list_active(&database).await.unwrap().is_empty());
    }
}
