use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use pillar_shared::proto::{
    CommandStreamRequest, ControllerCommand, LogAck, LogBatch, RegisterNodeRequest,
    RegisterNodeResponse, ReportStatusRequest, ReportStatusResponse, ScriptResult,
    ScriptResultAck,
};

use crate::alerts::AlertEngine;
use crate::db::{self, Db};
use crate::node_registry::NodeRegistry;

pub mod proto {
    tonic::include_proto!("pillar");
}

use proto::pillar_controller_server::PillarController;
pub use proto::pillar_controller_server::PillarControllerServer;

/// Validate the bearer token on incoming gRPC requests.
/// If `expected_token` is empty, auth is disabled (all requests pass).
pub fn check_auth_token(
    expected_token: &str,
    req: tonic::Request<()>,
) -> Result<tonic::Request<()>, Status> {
    if expected_token.is_empty() {
        return Ok(req);
    }
    match req.metadata().get("authorization") {
        Some(val) => {
            let val = val.to_str().unwrap_or("");
            let provided = val.strip_prefix("Bearer ").unwrap_or(val);
            if provided == expected_token {
                Ok(req)
            } else {
                Err(Status::unauthenticated("invalid auth token"))
            }
        }
        None => Err(Status::unauthenticated("missing authorization header")),
    }
}

pub struct GrpcServer {
    db: Db,
    registry: NodeRegistry,
    alert_engine: AlertEngine,
    /// IP extracted from external_url, used as fallback for local connections.
    self_ip: String,
}

impl GrpcServer {
    pub fn new(
        db: Db,
        registry: NodeRegistry,
        alert_engine: AlertEngine,
        external_url: &str,
    ) -> Self {
        let host = external_url
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        // Handle IPv6 in brackets [::1]:port, plain IPv4 1.2.3.4:port, or hostname:port
        let self_ip = if host.starts_with('[') {
            // IPv6: extract between brackets
            host.split(']').next().unwrap_or("").trim_start_matches('[').to_string()
        } else {
            // IPv4 or hostname: split on last colon (port)
            host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host).to_string()
        };
        Self {
            db,
            registry,
            alert_engine,
            self_ip,
        }
    }
}

#[tonic::async_trait]
impl PillarController for GrpcServer {
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeResponse>, Status> {
        let peer_ip = request
            .remote_addr()
            .map(|addr| addr.ip())
            .filter(|ip| !ip.is_loopback())
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| self.self_ip.clone());
        let req = request.into_inner();
        tracing::info!(node_id = %req.node_id, role = %req.role, client = %req.client, ip = %peer_ip, "RegisterNode");

        db::upsert_node(&self.db, &req, &peer_ip)
            .await
            .map_err(|e| Status::internal(format!("db error: {e}")))?;

        self.registry.register_node(&req.node_id).await;

        Ok(Response::new(RegisterNodeResponse {
            accepted: true,
            message: "registered".to_string(),
        }))
    }

    async fn report_status(
        &self,
        request: Request<ReportStatusRequest>,
    ) -> Result<Response<ReportStatusResponse>, Status> {
        let req = request.into_inner();
        let status = req
            .status
            .ok_or_else(|| Status::invalid_argument("missing status"))?;

        self.registry
            .update_status(&req.node_id, status.clone())
            .await;

        db::update_node_status(&self.db, &req.node_id, &status)
            .await
            .map_err(|e| Status::internal(format!("db error: {e}")))?;

        self.alert_engine.evaluate(&req.node_id, &status).await;

        Ok(Response::new(ReportStatusResponse {}))
    }

    type CommandStreamStream = ReceiverStream<Result<ControllerCommand, Status>>;

    async fn command_stream(
        &self,
        request: Request<CommandStreamRequest>,
    ) -> Result<Response<Self::CommandStreamStream>, Status> {
        let node_id = request.into_inner().node_id;
        tracing::info!(node_id = %node_id, "CommandStream opened");

        let rx = self.registry.create_command_channel(&node_id).await;

        // Wrap the mpsc::Receiver so each ControllerCommand becomes Ok(cmd).
        let (tx_out, rx_out) = tokio::sync::mpsc::channel::<Result<ControllerCommand, Status>>(32);

        tokio::spawn(async move {
            let mut rx = rx;
            while let Some(cmd) = rx.recv().await {
                if tx_out.send(Ok(cmd)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx_out)))
    }

    async fn report_script_result(
        &self,
        request: Request<ScriptResult>,
    ) -> Result<Response<ScriptResultAck>, Status> {
        let result = request.into_inner();

        if result.exit_code == 0 {
            tracing::info!(
                node_id = %result.node_id,
                script_id = %result.script_id,
                "script succeeded"
            );
        } else {
            tracing::warn!(
                node_id = %result.node_id,
                script_id = %result.script_id,
                exit_code = result.exit_code,
                timed_out = result.timed_out,
                error = %result.error,
                "script failed"
            );
        }

        // Update script execution record in DB
        if let Err(e) = db::complete_script_execution(
            &self.db,
            &result.script_id,
            result.exit_code,
            result.timed_out,
            &result.error,
        )
        .await
        {
            tracing::warn!(error = %e, "failed to update script execution record");
        }

        // Emit a controller log for the node
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let level = if result.exit_code == 0 {
            "info"
        } else {
            "error"
        };
        let message = if result.exit_code == 0 {
            format!("Script {} completed successfully", result.script_id)
        } else if result.timed_out {
            format!("Script {} timed out", result.script_id)
        } else {
            let detail = if !result.error.is_empty() {
                result.error.clone()
            } else {
                let source = if !result.stderr.is_empty() {
                    &result.stderr
                } else {
                    &result.stdout
                };
                let lines: Vec<&str> = source.trim().lines().collect();
                let start = lines.len().saturating_sub(10);
                lines[start..].join("\n")
            };
            format!(
                "Script {} failed (exit code {}):\n{}",
                result.script_id, result.exit_code, detail
            )
        };

        let entry = pillar_shared::proto::LogEntry {
            service: "controller".to_string(),
            level: level.to_string(),
            message,
            unit: String::new(),
            timestamp_unix_ms: now_ms,
        };
        self.registry
            .publish_logs(&result.node_id, std::slice::from_ref(&entry))
            .await;
        if let Err(e) =
            db::insert_logs(&self.db, &result.node_id, std::slice::from_ref(&entry)).await
        {
            tracing::warn!(error = %e, "failed to persist script result log");
        }

        Ok(Response::new(ScriptResultAck {}))
    }

    async fn push_logs(
        &self,
        request: Request<Streaming<LogBatch>>,
    ) -> Result<Response<LogAck>, Status> {
        let mut stream = request.into_inner();
        let mut total_count = 0u64;

        while let Some(batch) = stream
            .message()
            .await
            .map_err(|e| Status::internal(format!("stream error: {e}")))?
        {
            let node_id = &batch.node_id;
            let entries = &batch.entries;

            if !entries.is_empty() {
                let count = db::insert_logs(&self.db, node_id, entries)
                    .await
                    .map_err(|e| Status::internal(format!("db error: {e}")))?;
                total_count += count;

                self.registry.publish_logs(node_id, entries).await;
            }
        }

        Ok(Response::new(LogAck {
            received_count: total_count,
        }))
    }
}
