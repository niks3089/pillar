//! Hot-spare failover orchestration (Anza identity-swap model).
//!
//! A pair links a staked primary validator with an unstaked backup. Failover
//! moves only the voting identity: demote the primary to an unstaked keypair,
//! relay its tower file through the controller, promote the backup with
//! `set-identity --require-tower`. Crash failover skips the tower (primary is
//! dead) and demotes the ex-primary the moment its agent reconnects.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;

use pillar_shared::proto::{ExecuteScript, ScriptResult};

use crate::api;
use crate::db::{self, Db, FailoverOpRow, FailoverPairRow};
use crate::node_registry::NodeRegistry;
use crate::templates;

pub const DEFAULT_UNSTAKED_IDENTITY: &str = "/home/sol/unstaked-identity.json";
pub const DEFAULT_SYMLINK_PATH: &str = "/home/sol/pillar-identity.json";

const PREPARE_TIMEOUT_SECS: u32 = 300;
/// wait-for-restart-window legitimately takes minutes; nothing has changed on
/// the node until it returns, so a timeout here is safe.
const DEMOTE_TIMEOUT_SECS: u32 = 900;
const PROMOTE_TIMEOUT_SECS: u32 = 120;
const COLD_DEMOTE_TIMEOUT_SECS: u32 = 120;
/// An op with no script result after its longest script timeout plus this grace
/// is failed by the tick loop.
const OP_STALE_AFTER_SECS: i64 = DEMOTE_TIMEOUT_SECS as i64 + 60;
/// Heartbeat age after which a primary counts as dead for auto-failover.
const PRIMARY_DEAD_HEARTBEAT_SECS: i64 = 60;
/// Consecutive dead ticks (10s apart) before auto-failover fires.
const AUTO_FAILOVER_DEBOUNCE_TICKS: u32 = 3;
/// Max slots behind for a backup to be considered promotable.
const BACKUP_MAX_SLOTS_BEHIND: i64 = 50;
/// Minimum interval between cold-demote resends to a reconnecting ex-primary.
const COLD_DEMOTE_RESEND_SECS: u64 = 60;

const TOWER_BEGIN: &str = "---TOWER-B64-BEGIN---";
const TOWER_END: &str = "---TOWER-B64-END---";

pub struct CreatePairRequest {
    pub primary_node_id: String,
    pub backup_node_id: String,
    pub staked_identity_path: String,
    pub unstaked_identity_path: String,
    pub symlink_path: String,
    pub auto_failover: bool,
}

#[derive(Clone)]
pub struct FailoverEngine {
    db: Db,
    registry: NodeRegistry,
    /// node_id → consecutive ticks the primary has looked dead (auto-failover debounce).
    dead_ticks: Arc<DashMap<String, u32>>,
    /// node_id → pair_id for crashed ex-primaries awaiting demote-on-reconnect.
    pending_cold: Arc<DashMap<String, String>>,
    /// node_id → last cold-demote dispatch, for resend backoff.
    cold_sent_at: Arc<DashMap<String, Instant>>,
}

impl FailoverEngine {
    pub fn new(db: Db, registry: NodeRegistry) -> Self {
        Self {
            db,
            registry,
            dead_ticks: Arc::new(DashMap::new()),
            pending_cold: Arc::new(DashMap::new()),
            cold_sent_at: Arc::new(DashMap::new()),
        }
    }

    /// Reload pending cold-demotes after a controller restart.
    pub async fn load_pending(&self) {
        match db::list_pending_cold_demotes(&self.db).await {
            Ok(pairs) => {
                for pair in pairs {
                    if let Some(node_id) = pair.pending_cold_demote_node_id {
                        self.pending_cold.insert(node_id, pair.pair_id);
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to load pending cold demotes"),
        }
    }

    async fn log(&self, node_id: &str, level: &str, message: &str) {
        api::emit_controller_log(&self.registry, &self.db, node_id, level, message).await;
    }

    /// Send a script to a node and record it. Returns the script ID.
    async fn send_script(
        &self,
        node_id: &str,
        script: String,
        description: &str,
        timeout_secs: u32,
    ) -> Result<String, String> {
        let script_id = api::generate_script_id();
        let cmd = api::wrap_script(ExecuteScript {
            script_id: script_id.clone(),
            script,
            description: description.to_string(),
            timeout_secs,
        });
        self.registry.send_command(node_id, cmd).await?;
        if let Err(e) = db::insert_script_execution(&self.db, &script_id, node_id, description).await
        {
            tracing::warn!(error = %e, "failed to record failover script execution");
        }
        Ok(script_id)
    }

    // -----------------------------------------------------------------------
    // Pair creation / prepare
    // -----------------------------------------------------------------------

    pub async fn create_pair(&self, req: CreatePairRequest) -> Result<FailoverPairRow, String> {
        let pair = FailoverPairRow {
            pair_id: generate_pair_id(),
            primary_node_id: req.primary_node_id,
            backup_node_id: req.backup_node_id,
            staked_identity_path: req.staked_identity_path,
            unstaked_identity_path: req.unstaked_identity_path,
            symlink_path: req.symlink_path,
            staked_pubkey: None,
            auto_failover: req.auto_failover,
            prepare_state: "preparing".to_string(),
            prepare_primary_script_id: None,
            prepare_backup_script_id: None,
            prepare_error: None,
            pending_cold_demote_node_id: None,
            created_at: 0,
            updated_at: 0,
        };
        db::create_failover_pair(&self.db, &pair)
            .await
            .map_err(|e| format!("failed to create pair (node already paired?): {e}"))?;
        self.dispatch_prepare(&pair.pair_id).await?;
        db::get_failover_pair(&self.db, &pair.pair_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "pair vanished after create".to_string())
    }

    /// Render and send the prepare script to both nodes; resets prepare state.
    pub async fn dispatch_prepare(&self, pair_id: &str) -> Result<(), String> {
        let pair = self.load_pair(pair_id).await?;

        let mut script_ids = Vec::with_capacity(2);
        for is_primary in [true, false] {
            let node_id = if is_primary {
                &pair.primary_node_id
            } else {
                &pair.backup_node_id
            };
            let config = db::get_provision_config(&self.db, node_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("node {node_id} has no provision config"))?;
            let mut vars = api::failover_prepare_vars(
                &config,
                &pair.symlink_path,
                &pair.staked_identity_path,
            )?;
            let role = if is_primary { "primary" } else { "backup" };
            let symlink_target = if is_primary {
                &pair.staked_identity_path
            } else {
                &pair.unstaked_identity_path
            };
            // The backup votes with a junk identity, so restarting it into the new
            // unit is free. The primary keeps running; its unit applies at the next
            // natural restart and demotion works on the live process via set-identity.
            let restart_section = if is_primary {
                "echo \"Primary left running; new unit applies at next restart\"".to_string()
            } else {
                let service = vars.get("service_name").cloned().unwrap_or_default();
                format!("sudo systemctl restart {service}\necho \"Backup restarted with unstaked identity\"")
            };
            vars.insert("role".to_string(), role.to_string());
            vars.insert("staked_identity_path".to_string(), pair.staked_identity_path.clone());
            vars.insert("unstaked_identity_path".to_string(), pair.unstaked_identity_path.clone());
            vars.insert("symlink_path".to_string(), pair.symlink_path.clone());
            vars.insert("symlink_target".to_string(), symlink_target.clone());
            vars.insert("restart_section".to_string(), restart_section);

            let script = templates::render(templates::scripts::FAILOVER_PREPARE, &vars);
            let script_id = self
                .send_script(
                    node_id,
                    script,
                    &format!("Failover prepare ({role})"),
                    PREPARE_TIMEOUT_SECS,
                )
                .await
                .map_err(|e| format!("failed to send prepare to {node_id}: {e}"))?;
            script_ids.push(script_id);
        }

        db::set_pair_prepare_scripts(&self.db, pair_id, &script_ids[0], &script_ids[1])
            .await
            .map_err(|e| e.to_string())?;
        self.log(
            &pair.primary_node_id,
            "info",
            "Failover prepare dispatched to primary and backup",
        )
        .await;
        Ok(())
    }

    /// Called by provision_node: a re-provisioned node's unit no longer matches
    /// the failover setup, so the pair must be re-prepared.
    pub async fn invalidate_on_provision(&self, node_id: &str) {
        if let Ok(Some(pair)) = db::get_failover_pair_by_node(&self.db, node_id).await {
            let _ = db::set_pair_prepare_state(
                &self.db,
                &pair.pair_id,
                "prepare_failed",
                Some("node re-provisioned; re-run failover prepare"),
            )
            .await;
        }
    }

    // -----------------------------------------------------------------------
    // Failover triggers
    // -----------------------------------------------------------------------

    /// Graceful swap: demote the live primary, relay the tower, promote the backup.
    pub async fn trigger_graceful(&self, pair_id: &str) -> Result<FailoverOpRow, String> {
        let pair = self.load_pair(pair_id).await?;
        self.check_no_active_op(&pair).await?;
        if pair.prepare_state != "ready" {
            return Err(format!(
                "pair is not ready for failover (prepare state: {})",
                pair.prepare_state
            ));
        }
        let staked_pubkey = pair
            .staked_pubkey
            .clone()
            .ok_or("pair has no staked pubkey recorded; re-run prepare")?;
        self.check_backup_promotable(&pair.backup_node_id).await?;

        let summary = self.node_summary(&pair.primary_node_id).await?;
        let vars = HashMap::from([
            ("binary_path".to_string(), summary.binary_path),
            ("ledger_path".to_string(), summary.ledger_path),
            ("symlink_path".to_string(), pair.symlink_path.clone()),
            ("unstaked_identity_path".to_string(), pair.unstaked_identity_path.clone()),
            ("staked_pubkey".to_string(), staked_pubkey),
        ]);
        let script = templates::render(templates::scripts::FAILOVER_DEMOTE, &vars);
        let script_id = self
            .send_script(
                &pair.primary_node_id,
                script,
                "Failover: demote primary",
                DEMOTE_TIMEOUT_SECS,
            )
            .await?;

        let op = FailoverOpRow {
            op_id: generate_op_id(),
            pair_id: pair.pair_id.clone(),
            kind: "graceful".to_string(),
            state: "pending_demote".to_string(),
            from_node_id: pair.primary_node_id.clone(),
            to_node_id: pair.backup_node_id.clone(),
            demote_script_id: Some(script_id),
            promote_script_id: None,
            cold_demote_script_id: None,
            tower_b64: None,
            error: None,
            started_at: 0,
            updated_at: 0,
            completed_at: None,
        };
        db::insert_failover_op(&self.db, &op)
            .await
            .map_err(|e| e.to_string())?;
        self.log(
            &pair.primary_node_id,
            "warn",
            &format!(
                "Failover started (graceful): {} → {}",
                pair.primary_node_id, pair.backup_node_id
            ),
        )
        .await;
        Ok(op)
    }

    /// Crash failover: promote the backup WITHOUT a tower file. Requires the
    /// primary to look dead unless `force` is set (operator override).
    pub async fn trigger_crash(&self, pair_id: &str, force: bool) -> Result<FailoverOpRow, String> {
        let pair = self.load_pair(pair_id).await?;
        self.check_no_active_op(&pair).await?;
        if pair.prepare_state != "ready" {
            return Err(format!(
                "pair is not ready for failover (prepare state: {})",
                pair.prepare_state
            ));
        }
        if !force && !self.primary_looks_dead(&pair.primary_node_id).await {
            return Err(
                "primary does not look dead; use a graceful failover, or force to override"
                    .to_string(),
            );
        }
        self.check_backup_promotable(&pair.backup_node_id).await?;

        let script_id = self.send_promote(&pair, None).await?;
        let op = FailoverOpRow {
            op_id: generate_op_id(),
            pair_id: pair.pair_id.clone(),
            kind: "crash".to_string(),
            state: "pending_promote".to_string(),
            from_node_id: pair.primary_node_id.clone(),
            to_node_id: pair.backup_node_id.clone(),
            demote_script_id: None,
            promote_script_id: Some(script_id),
            cold_demote_script_id: None,
            tower_b64: None,
            error: None,
            started_at: 0,
            updated_at: 0,
            completed_at: None,
        };
        db::insert_failover_op(&self.db, &op)
            .await
            .map_err(|e| e.to_string())?;
        self.log(
            &pair.backup_node_id,
            "warn",
            &format!(
                "Crash failover started: promoting {} without tower (primary {} down)",
                pair.backup_node_id, pair.primary_node_id
            ),
        )
        .await;
        Ok(op)
    }

    /// Render and send the promote script to the backup. `tower_b64` present =
    /// graceful path with --require-tower; absent = towerless crash promote.
    async fn send_promote(
        &self,
        pair: &FailoverPairRow,
        tower_b64: Option<&str>,
    ) -> Result<String, String> {
        let summary = self.node_summary(&pair.backup_node_id).await?;
        let staked_pubkey = pair.staked_pubkey.clone().unwrap_or_default();
        let (tower_section, require_tower_flag) = match tower_b64 {
            Some(b64) => (
                format!(
                    "TOWER_PATH=\"{ledger}/tower-1_9-{pk}.bin\"\n\
                     printf '%s' '{b64}' | base64 -d > \"$TOWER_PATH\"\n\
                     echo \"Tower restored to $TOWER_PATH\"",
                    ledger = summary.ledger_path,
                    pk = staked_pubkey,
                ),
                " --require-tower".to_string(),
            ),
            None => (
                "echo \"No tower file (crash failover)\"".to_string(),
                String::new(),
            ),
        };
        let vars = HashMap::from([
            ("binary_path".to_string(), summary.binary_path),
            ("ledger_path".to_string(), summary.ledger_path),
            ("symlink_path".to_string(), pair.symlink_path.clone()),
            ("staked_identity_path".to_string(), pair.staked_identity_path.clone()),
            ("staked_pubkey".to_string(), staked_pubkey),
            ("tower_section".to_string(), tower_section),
            ("require_tower_flag".to_string(), require_tower_flag),
        ]);
        let script = templates::render(templates::scripts::FAILOVER_PROMOTE, &vars);
        self.send_script(
            &pair.backup_node_id,
            script,
            "Failover: promote backup",
            PROMOTE_TIMEOUT_SECS,
        )
        .await
    }

    // -----------------------------------------------------------------------
    // Script result handling (hooked from grpc_server::report_script_result)
    // -----------------------------------------------------------------------

    pub async fn on_script_result(&self, result: &ScriptResult) {
        // Prepare scripts are tracked on the pair row.
        if let Ok(Some(pair)) = db::get_failover_pair_by_node(&self.db, &result.node_id).await {
            let is_primary = pair.prepare_primary_script_id.as_deref() == Some(&result.script_id);
            let is_backup = pair.prepare_backup_script_id.as_deref() == Some(&result.script_id);
            if is_primary || is_backup {
                self.handle_prepare_result(pair, result, is_primary).await;
                return;
            }
        }
        if let Ok(Some(op)) = db::get_op_by_script_id(&self.db, &result.script_id).await {
            self.handle_op_result(op, result).await;
        }
    }

    async fn handle_prepare_result(
        &self,
        pair: FailoverPairRow,
        result: &ScriptResult,
        is_primary: bool,
    ) {
        if result.exit_code != 0 {
            let detail = tail_of(&result.stderr, &result.stdout);
            let _ = db::set_pair_prepare_state(
                &self.db,
                &pair.pair_id,
                "prepare_failed",
                Some(&format!("prepare failed on {}: {detail}", result.node_id)),
            )
            .await;
            return;
        }

        let pubkey = match parse_pubkey_sentinel(&result.stdout) {
            Some(pk) => pk,
            None => {
                let _ = db::set_pair_prepare_state(
                    &self.db,
                    &pair.pair_id,
                    "prepare_failed",
                    Some(&format!(
                        "prepare on {} did not report the staked pubkey",
                        result.node_id
                    )),
                )
                .await;
                return;
            }
        };

        // Both nodes must hold the SAME staked identity (copied out-of-band).
        match &pair.staked_pubkey {
            None => {
                let _ = db::set_pair_staked_pubkey(&self.db, &pair.pair_id, &pubkey).await;
            }
            Some(existing) if existing != &pubkey => {
                let _ = db::set_pair_prepare_state(
                    &self.db,
                    &pair.pair_id,
                    "prepare_failed",
                    Some("staked identity differs between primary and backup"),
                )
                .await;
                return;
            }
            Some(_) => {}
        }

        let _ = db::advance_pair_prepare(&self.db, &pair.pair_id, is_primary).await;
        if let Ok(Some(updated)) = db::get_failover_pair(&self.db, &pair.pair_id).await {
            if updated.prepare_state == "ready" {
                self.log(
                    &updated.primary_node_id,
                    "info",
                    &format!(
                        "Failover pair ready: {} (primary) ↔ {} (backup), identity {}",
                        updated.primary_node_id,
                        updated.backup_node_id,
                        updated.staked_pubkey.as_deref().unwrap_or("?")
                    ),
                )
                .await;
            }
        }
    }

    async fn handle_op_result(&self, op: FailoverOpRow, result: &ScriptResult) {
        if op.cold_demote_script_id.as_deref() == Some(&result.script_id) {
            self.handle_cold_demote_result(op, result).await;
        } else if op.demote_script_id.as_deref() == Some(&result.script_id) {
            self.handle_demote_result(op, result).await;
        } else if op.promote_script_id.as_deref() == Some(&result.script_id) {
            self.handle_promote_result(op, result).await;
        }
    }

    async fn handle_demote_result(&self, op: FailoverOpRow, result: &ScriptResult) {
        if op.state != "pending_demote" {
            return;
        }
        let tower = parse_tower_b64(&result.stdout);

        if result.exit_code != 0 {
            // If set-identity already ran (tower block emitted), the primary has
            // been demoted — stalling now would leave nobody voting, so push on.
            if let TowerParse::Found(ref b64) = tower {
                tracing::warn!(
                    op_id = %op.op_id,
                    "demote script failed after identity swap; continuing to promote"
                );
                self.continue_to_promote(op, b64.clone()).await;
                return;
            }
            let detail = tail_of(&result.stderr, &result.stdout);
            let _ = db::update_op_state(
                &self.db,
                &op.op_id,
                "failed",
                Some(&format!("demote failed on {}: {detail}", op.from_node_id)),
            )
            .await;
            self.log(
                &op.from_node_id,
                "error",
                "Failover aborted: demote failed, primary unchanged",
            )
            .await;
            return;
        }

        match tower {
            TowerParse::Found(b64) => self.continue_to_promote(op, b64).await,
            TowerParse::EmptySentinel | TowerParse::Absent => {
                // Primary IS demoted but we can't hand over safely. Deliberate
                // friction: the operator must explicitly promote without tower.
                let _ = db::update_op_state(
                    &self.db,
                    &op.op_id,
                    "failed",
                    Some(
                        "primary demoted but tower file missing; \
                         use crash failover with force to promote without tower",
                    ),
                )
                .await;
                self.log(
                    &op.from_node_id,
                    "error",
                    "Failover halted: primary demoted but tower file missing on primary",
                )
                .await;
            }
        }
    }

    async fn continue_to_promote(&self, op: FailoverOpRow, tower_b64: String) {
        let pair = match db::get_failover_pair(&self.db, &op.pair_id).await {
            Ok(Some(p)) => p,
            _ => {
                let _ = db::update_op_state(&self.db, &op.op_id, "failed", Some("pair deleted"))
                    .await;
                return;
            }
        };
        match self.send_promote(&pair, Some(&tower_b64)).await {
            Ok(script_id) => {
                let _ =
                    db::set_op_promote(&self.db, &op.op_id, &script_id, Some(&tower_b64)).await;
                self.log(
                    &op.to_node_id,
                    "info",
                    "Failover: tower relayed, promoting backup",
                )
                .await;
            }
            Err(e) => {
                let _ = db::update_op_state(
                    &self.db,
                    &op.op_id,
                    "failed",
                    Some(&format!(
                        "primary demoted but promote dispatch failed: {e}; \
                         re-trigger with crash failover (force) once the backup is reachable"
                    )),
                )
                .await;
                self.log(&op.to_node_id, "error", "Failover: promote dispatch failed").await;
            }
        }
    }

    async fn handle_promote_result(&self, op: FailoverOpRow, result: &ScriptResult) {
        if op.state != "pending_promote" {
            return;
        }
        if result.exit_code != 0 {
            let detail = tail_of(&result.stderr, &result.stdout);
            let _ = db::update_op_state(
                &self.db,
                &op.op_id,
                "failed",
                Some(&format!(
                    "promote failed on {}: {detail}; staked identity is currently NOT voting \
                     — retry with crash failover (force)",
                    op.to_node_id
                )),
            )
            .await;
            self.log(&op.to_node_id, "error", "Failover: promote failed").await;
            return;
        }

        let _ = db::update_op_state(&self.db, &op.op_id, "complete", None).await;
        // The old backup is now the staked voter; swap roles so the pair stays
        // truthful and the next failover runs the other way.
        let _ = db::swap_pair_roles(&self.db, &op.pair_id).await;
        if op.kind == "crash" {
            let _ =
                db::set_pair_pending_cold_demote(&self.db, &op.pair_id, Some(&op.from_node_id))
                    .await;
            self.pending_cold
                .insert(op.from_node_id.clone(), op.pair_id.clone());
        }
        self.log(
            &op.to_node_id,
            "info",
            &format!(
                "Failover complete: {} is now the staked voter (was {})",
                op.to_node_id, op.from_node_id
            ),
        )
        .await;
    }

    async fn handle_cold_demote_result(&self, op: FailoverOpRow, result: &ScriptResult) {
        if result.exit_code == 0 {
            let _ = db::set_pair_pending_cold_demote(&self.db, &op.pair_id, None).await;
            self.pending_cold.remove(&op.from_node_id);
            self.cold_sent_at.remove(&op.from_node_id);
            self.log(
                &op.from_node_id,
                "info",
                "Recovered ex-primary demoted to unstaked identity",
            )
            .await;
        } else {
            // Keep it pending; on_node_seen will resend after the backoff.
            self.log(
                &op.from_node_id,
                "error",
                "Cold demote of recovered ex-primary failed; will retry",
            )
            .await;
        }
    }

    // -----------------------------------------------------------------------
    // Reconnect + periodic tick (hooked from report_status and main loop)
    // -----------------------------------------------------------------------

    /// Called on every status report. Cheap: a map lookup unless the node is a
    /// crashed ex-primary that owes us a cold demote.
    pub async fn on_node_seen(&self, node_id: &str) {
        let pair_id = match self.pending_cold.get(node_id) {
            Some(entry) => entry.value().clone(),
            None => return,
        };
        if let Some(sent) = self.cold_sent_at.get(node_id) {
            if sent.elapsed() < Duration::from_secs(COLD_DEMOTE_RESEND_SECS) {
                return;
            }
        }
        self.cold_sent_at.insert(node_id.to_string(), Instant::now());

        let pair = match db::get_failover_pair(&self.db, &pair_id).await {
            Ok(Some(p)) if p.pending_cold_demote_node_id.as_deref() == Some(node_id) => p,
            _ => {
                self.pending_cold.remove(node_id);
                return;
            }
        };
        let summary = match self.node_summary(node_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(node_id, error = %e, "cannot render cold demote");
                return;
            }
        };
        let vars = HashMap::from([
            ("service_name".to_string(), summary.service_name),
            ("binary_path".to_string(), summary.binary_path),
            ("ledger_path".to_string(), summary.ledger_path),
            ("symlink_path".to_string(), pair.symlink_path.clone()),
            ("unstaked_identity_path".to_string(), pair.unstaked_identity_path.clone()),
        ]);
        let script = templates::render(templates::scripts::FAILOVER_DEMOTE_COLD, &vars);
        match self
            .send_script(
                node_id,
                script,
                "Failover: demote recovered ex-primary",
                COLD_DEMOTE_TIMEOUT_SECS,
            )
            .await
        {
            Ok(script_id) => {
                if let Ok(Some(op)) = db::get_latest_op_for_pair(&self.db, &pair_id).await {
                    let _ = db::set_op_cold_demote_script(&self.db, &op.op_id, &script_id).await;
                }
                self.log(
                    node_id,
                    "warn",
                    "Crashed ex-primary reconnected; demoting to unstaked identity",
                )
                .await;
            }
            Err(e) => tracing::warn!(node_id, error = %e, "failed to send cold demote"),
        }
    }

    /// Periodic maintenance: fail stale ops, run the auto-failover monitor.
    pub async fn tick(&self) {
        match db::fail_stale_ops(&self.db, OP_STALE_AFTER_SECS).await {
            Ok(failed) => {
                for op in failed {
                    self.log(
                        &op.from_node_id,
                        "error",
                        &format!("Failover op {} timed out waiting for script result", op.op_id),
                    )
                    .await;
                }
            }
            Err(e) => tracing::warn!(error = %e, "fail_stale_ops error"),
        }

        let pairs = match db::list_failover_pairs(&self.db).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "failover tick: list pairs failed");
                return;
            }
        };
        for pair in pairs {
            if !pair.auto_failover || pair.prepare_state != "ready" {
                self.dead_ticks.remove(&pair.primary_node_id);
                continue;
            }
            if let Ok(Some(_)) = db::get_active_op_for_pair(&self.db, &pair.pair_id).await {
                continue;
            }

            if !self.primary_looks_dead(&pair.primary_node_id).await {
                self.dead_ticks.remove(&pair.primary_node_id);
                continue;
            }
            let ticks = {
                let mut entry = self
                    .dead_ticks
                    .entry(pair.primary_node_id.clone())
                    .or_insert(0);
                *entry += 1;
                *entry
            };
            if ticks < AUTO_FAILOVER_DEBOUNCE_TICKS {
                continue;
            }
            if self.check_backup_promotable(&pair.backup_node_id).await.is_err() {
                continue;
            }

            self.dead_ticks.remove(&pair.primary_node_id);
            self.log(
                &pair.primary_node_id,
                "error",
                &format!(
                    "AUTO-FAILOVER: primary {} is down, promoting backup {}",
                    pair.primary_node_id, pair.backup_node_id
                ),
            )
            .await;
            if let Err(e) = self.trigger_crash(&pair.pair_id, false).await {
                tracing::warn!(pair_id = %pair.pair_id, error = %e, "auto-failover trigger failed");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Checks + lookups
    // -----------------------------------------------------------------------

    async fn load_pair(&self, pair_id: &str) -> Result<FailoverPairRow, String> {
        db::get_failover_pair(&self.db, pair_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("failover pair not found: {pair_id}"))
    }

    async fn check_no_active_op(&self, pair: &FailoverPairRow) -> Result<(), String> {
        match db::get_active_op_for_pair(&self.db, &pair.pair_id).await {
            Ok(Some(op)) => Err(format!("failover already in progress ({})", op.state)),
            Ok(None) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn check_backup_promotable(&self, backup_node_id: &str) -> Result<(), String> {
        let status = self
            .registry
            .get_status(backup_node_id)
            .await
            .ok_or_else(|| format!("backup {backup_node_id} has no live status"))?;
        let node = db::get_node(&self.db, backup_node_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("backup {backup_node_id} not found"))?;
        let age = node
            .last_seen_at
            .map(|t| now_epoch_secs() - t)
            .unwrap_or(i64::MAX);
        if age > PRIMARY_DEAD_HEARTBEAT_SECS {
            return Err(format!("backup {backup_node_id} is not reporting (last seen {age}s ago)"));
        }
        if !backup_state_promotable(&status.state, status.slots_behind) {
            return Err(format!(
                "backup {} is not promotable (state: {}, {} slots behind)",
                backup_node_id, status.state, status.slots_behind
            ));
        }
        Ok(())
    }

    async fn primary_looks_dead(&self, primary_node_id: &str) -> bool {
        let age = match db::get_node(&self.db, primary_node_id).await {
            Ok(Some(node)) => node
                .last_seen_at
                .map(|t| now_epoch_secs() - t)
                .unwrap_or(i64::MAX),
            _ => i64::MAX,
        };
        let status = self.registry.get_status(primary_node_id).await;
        is_primary_dead(
            age,
            status.as_ref().map(|s| s.state.as_str()),
            status.as_ref().map(|s| s.crash_looping).unwrap_or(false),
        )
    }

    async fn node_summary(&self, node_id: &str) -> Result<api::ProvisionSummary, String> {
        let config = db::get_provision_config(&self.db, node_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("node {node_id} has no provision config"))?;
        api::provision_summary(&config)
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

fn now_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn generate_pair_id() -> String {
    format!("fo-{}", api::generate_script_id().trim_start_matches("script-"))
}

fn generate_op_id() -> String {
    format!("op-{}", api::generate_script_id().trim_start_matches("script-"))
}

#[derive(Debug, PartialEq)]
pub enum TowerParse {
    /// Validated base64 tower blob.
    Found(String),
    /// Sentinels present but no tower file existed on the primary.
    EmptySentinel,
    /// No sentinel block in the output at all.
    Absent,
}

/// Extract the tower blob relayed between sentinels in demote-script stdout.
///
/// SECURITY: the returned string is later embedded into the promote script that
/// runs on the backup, so this charset check is the injection boundary for
/// agent-originated data. Anything outside strict base64 is rejected.
pub fn parse_tower_b64(stdout: &str) -> TowerParse {
    let Some(begin) = stdout.find(TOWER_BEGIN) else {
        return TowerParse::Absent;
    };
    let after = &stdout[begin + TOWER_BEGIN.len()..];
    let Some(end) = after.find(TOWER_END) else {
        return TowerParse::Absent;
    };
    let blob: String = after[..end].chars().filter(|c| !c.is_whitespace()).collect();
    if blob.is_empty() {
        return TowerParse::EmptySentinel;
    }
    if !blob
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return TowerParse::EmptySentinel;
    }
    TowerParse::Found(blob)
}

/// Extract the staked pubkey from prepare-script stdout. Strict base58 check:
/// the value lands in a tower filename on the backup.
pub fn parse_pubkey_sentinel(stdout: &str) -> Option<String> {
    const BEGIN: &str = "---PILLAR-PUBKEY:";
    let start = stdout.find(BEGIN)? + BEGIN.len();
    let end = stdout[start..].find("---")?;
    let pk = &stdout[start..start + end];
    let valid = (32..=44).contains(&pk.len())
        && pk.chars().all(|c| {
            c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l')
        });
    if valid {
        Some(pk.to_string())
    } else {
        None
    }
}

/// A primary is dead when its agent stopped reporting (machine/agent down) or
/// it reports the validator as off / crash-looping.
pub fn is_primary_dead(heartbeat_age_secs: i64, raw_state: Option<&str>, crash_looping: bool) -> bool {
    heartbeat_age_secs > PRIMARY_DEAD_HEARTBEAT_SECS
        || raw_state == Some("off")
        || crash_looping
}

/// A backup can take the staked identity when it's healthy or only slightly behind.
pub fn backup_state_promotable(state: &str, slots_behind: i64) -> bool {
    match state {
        "healthy" => true,
        "behind" => slots_behind <= BACKUP_MAX_SLOTS_BEHIND,
        _ => false,
    }
}

fn tail_of(stderr: &str, stdout: &str) -> String {
    let source = if !stderr.trim().is_empty() { stderr } else { stdout };
    let lines: Vec<&str> = source.trim().lines().collect();
    let start = lines.len().saturating_sub(5);
    lines[start..].join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tower_parse_roundtrip() {
        let stdout = format!("demoting...\n{TOWER_BEGIN}\ndG93ZXJkYXRhCg==\n{TOWER_END}\ndone");
        assert_eq!(
            parse_tower_b64(&stdout),
            TowerParse::Found("dG93ZXJkYXRhCg==".to_string())
        );
    }

    #[test]
    fn tower_parse_empty_sentinel() {
        let stdout = format!("{TOWER_BEGIN}\n\n{TOWER_END}");
        assert_eq!(parse_tower_b64(&stdout), TowerParse::EmptySentinel);
    }

    #[test]
    fn tower_parse_absent() {
        assert_eq!(parse_tower_b64("no sentinels here"), TowerParse::Absent);
        let unterminated = format!("{TOWER_BEGIN}\nabc");
        assert_eq!(parse_tower_b64(&unterminated), TowerParse::Absent);
    }

    #[test]
    fn tower_parse_rejects_injection() {
        // Anything outside the base64 charset must be rejected — this string
        // would otherwise be embedded in a script run on the backup.
        let evil = format!("{TOWER_BEGIN}\n'; rm -rf / #\n{TOWER_END}");
        assert_eq!(parse_tower_b64(&evil), TowerParse::EmptySentinel);
        let subshell = format!("{TOWER_BEGIN}\naGk=$(reboot)\n{TOWER_END}");
        assert_eq!(parse_tower_b64(&subshell), TowerParse::EmptySentinel);
    }

    #[test]
    fn pubkey_sentinel_parse() {
        let stdout = "prep done\n---PILLAR-PUBKEY:Fd7btgySsrjuo25CJCj7oE7VPMyezDhnx7pZkj2v69Nk---\n";
        assert_eq!(
            parse_pubkey_sentinel(stdout).as_deref(),
            Some("Fd7btgySsrjuo25CJCj7oE7VPMyezDhnx7pZkj2v69Nk")
        );
        // Injection / malformed values are rejected
        assert!(parse_pubkey_sentinel("---PILLAR-PUBKEY:$(reboot)---").is_none());
        assert!(parse_pubkey_sentinel("---PILLAR-PUBKEY:short---").is_none());
        assert!(parse_pubkey_sentinel("no sentinel").is_none());
    }

    #[test]
    fn primary_dead_truth_table() {
        // Fresh heartbeat, healthy state → alive
        assert!(!is_primary_dead(5, Some("healthy"), false));
        // Fresh heartbeat but validator off → dead (agent alive, validator down)
        assert!(is_primary_dead(5, Some("off"), false));
        // Crash looping → dead
        assert!(is_primary_dead(5, Some("starting_up"), true));
        // Stale heartbeat → dead regardless of last reported state
        assert!(is_primary_dead(120, Some("healthy"), false));
        // No status at all but fresh heartbeat → alive (just registered)
        assert!(!is_primary_dead(5, None, false));
    }

    #[test]
    fn backup_promotable_states() {
        assert!(backup_state_promotable("healthy", 0));
        assert!(backup_state_promotable("behind", 10));
        assert!(!backup_state_promotable("behind", 500));
        assert!(!backup_state_promotable("off", 0));
        assert!(!backup_state_promotable("starting_up", 0));
        assert!(!backup_state_promotable("recovering", 0));
    }

    #[test]
    fn templates_render_without_residue() {
        // Any leftover {{...}} means a var-name typo between engine and template.
        let demote_vars = HashMap::from([
            ("binary_path".to_string(), "/usr/local/bin/agave-validator".to_string()),
            ("ledger_path".to_string(), "/mnt/ledger".to_string()),
            ("symlink_path".to_string(), "/home/sol/pillar-identity.json".to_string()),
            ("unstaked_identity_path".to_string(), "/home/sol/unstaked.json".to_string()),
            ("staked_pubkey".to_string(), "Fd7btgySsrjuo25CJCj7oE7VPMyezDhnx7pZkj2v69Nk".to_string()),
        ]);
        let rendered = templates::render(templates::scripts::FAILOVER_DEMOTE, &demote_vars);
        assert!(!rendered.contains("{{"), "unrendered placeholder in demote:\n{rendered}");
        assert!(rendered.contains("wait-for-restart-window"));
        assert!(rendered.contains("tower-1_9-Fd7btgySsrjuo25CJCj7oE7VPMyezDhnx7pZkj2v69Nk.bin"));

        let promote_vars = HashMap::from([
            ("binary_path".to_string(), "/usr/local/bin/agave-validator".to_string()),
            ("ledger_path".to_string(), "/mnt/ledger".to_string()),
            ("symlink_path".to_string(), "/home/sol/pillar-identity.json".to_string()),
            ("staked_identity_path".to_string(), "/home/sol/staked.json".to_string()),
            ("staked_pubkey".to_string(), "pk".to_string()),
            ("tower_section".to_string(), "echo tower".to_string()),
            ("require_tower_flag".to_string(), " --require-tower".to_string()),
        ]);
        let rendered = templates::render(templates::scripts::FAILOVER_PROMOTE, &promote_vars);
        assert!(!rendered.contains("{{"), "unrendered placeholder in promote:\n{rendered}");
        assert!(rendered.contains("set-identity --require-tower /home/sol/staked.json"));

        let cold_vars = HashMap::from([
            ("service_name".to_string(), "solana-validator".to_string()),
            ("binary_path".to_string(), "/usr/local/bin/agave-validator".to_string()),
            ("ledger_path".to_string(), "/mnt/ledger".to_string()),
            ("symlink_path".to_string(), "/home/sol/pillar-identity.json".to_string()),
            ("unstaked_identity_path".to_string(), "/home/sol/unstaked.json".to_string()),
        ]);
        let rendered = templates::render(templates::scripts::FAILOVER_DEMOTE_COLD, &cold_vars);
        assert!(!rendered.contains("{{"), "unrendered placeholder in cold demote:\n{rendered}");
        // Safety property: the symlink flip must come before any service interaction.
        let flip = rendered.find("ln -sfn").unwrap();
        let svc = rendered.find("systemctl").unwrap();
        assert!(flip < svc, "cold demote must flip symlink before touching the service");
    }
}
