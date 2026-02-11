use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

use pillar_shared::proto::{
    CommandStreamRequest, ControllerCommand, LogAck, LogBatch, RegisterNodeRequest,
    RegisterNodeResponse, ReportStatusRequest, ReportStatusResponse, UpgradeStatusRequest,
    UpgradeStatusResponse,
};

use crate::db::{self, Db};
use crate::node_registry::NodeRegistry;

pub mod proto {
    tonic::include_proto!("pillar");
}

use proto::pillar_controller_server::PillarController;
pub use proto::pillar_controller_server::PillarControllerServer;

pub struct GrpcServer {
    db: Db,
    registry: NodeRegistry,
    /// IP extracted from external_url, used as fallback for local connections.
    self_ip: String,
}

impl GrpcServer {
    pub fn new(db: Db, registry: NodeRegistry, external_url: &str) -> Self {
        let self_ip = external_url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split(':')
            .next()
            .unwrap_or("")
            .to_string();
        Self {
            db,
            registry,
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

    async fn report_upgrade_status(
        &self,
        request: Request<UpgradeStatusRequest>,
    ) -> Result<Response<UpgradeStatusResponse>, Status> {
        let req = request.into_inner();
        if req.success {
            tracing::info!(
                node_id = %req.node_id,
                binary = %req.binary_name,
                version = %req.version,
                "upgrade succeeded"
            );
        } else {
            tracing::warn!(
                node_id = %req.node_id,
                binary = %req.binary_name,
                version = %req.version,
                error = %req.error_message,
                "upgrade failed"
            );
        }
        Ok(Response::new(UpgradeStatusResponse {}))
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
