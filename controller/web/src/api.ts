export interface NodeStatus {
  state: string
  local_slot: number
  reference_slot: number
  slots_behind: number
  healthy: boolean
  restart_count: number
  crash_looping: boolean
  version: string
  role: string
  client: string
  cluster: string
  cpu_usage_percent: number
  memory_used_bytes: number
  memory_total_bytes: number
  disk_used_bytes: number
  disk_total_bytes: number
  updated_at_unix_secs: number
  state_duration_secs: number
  hostname: string
}

export interface Node {
  node_id: string
  lifecycle_state: string
  role?: string
  client?: string
  cluster?: string
  hostname?: string
  ip_address?: string
  agent_version?: string
  last_seen_at?: number
  registered_at?: number
  provision_config_json?: string
  live_status?: NodeStatus
}

export interface FleetOverview {
  total: number
  by_state: Record<string, number>
}

export interface LogEntry {
  id: number
  service: string
  level: string
  message: string
  unit?: string
  timestamp_ms: number
}

async function api<T>(path: string, opts?: RequestInit): Promise<T> {
  const res = await fetch(path, opts)
  if (res.status === 401) {
    window.location.reload()
    throw new Error('Session expired')
  }
  if (!res.ok) {
    const text = await res.text()
    throw new Error(`${res.status}: ${text}`)
  }
  return res.json()
}

export async function fetchOverview(): Promise<FleetOverview> {
  return api('/api/overview')
}

export async function fetchNodes(): Promise<Node[]> {
  return api('/api/nodes')
}

export async function fetchNode(id: string): Promise<Node> {
  return api(`/api/nodes/${encodeURIComponent(id)}`)
}

export async function fetchNodeLogs(
  id: string,
  params?: { service?: string; level?: string; since?: number; limit?: number },
): Promise<LogEntry[]> {
  const qs = new URLSearchParams()
  if (params?.service) qs.set('service', params.service)
  if (params?.level) qs.set('level', params.level)
  if (params?.since) qs.set('since', String(params.since))
  if (params?.limit) qs.set('limit', String(params.limit))
  const query = qs.toString()
  return api(`/api/nodes/${encodeURIComponent(id)}/logs${query ? '?' + query : ''}`)
}

export async function fetchOnboardCommand(): Promise<{ command: string }> {
  return api('/api/onboard-command')
}

export async function restartNode(id: string): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/restart`, { method: 'POST' })
}

export async function recoverNode(id: string): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/recover`, { method: 'POST' })
}

export async function deleteNode(id: string): Promise<void> {
  const res = await fetch(`/api/nodes/${encodeURIComponent(id)}`, { method: 'DELETE' })
  if (res.status === 401) {
    window.location.reload()
    throw new Error('Session expired')
  }
}

export async function stopNode(id: string): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/stop`, { method: 'POST' })
}

export async function cancelDeployment(id: string): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/cancel`, { method: 'POST' })
}

export interface ProvisionRequest {
  client: string
  version: string
  cluster: string
  identity_keypair_path: string
  vote_account_keypair_path: string
  ledger_path: string
  snapshot_path: string
  accounts_path: string
  entrypoints: string[]
  known_validators: string[]
  download_url: string
  sha256: string
  jito_mev: boolean
  jito_block_engine_url: string
  jito_relayer_url?: string
  jito_shred_receiver_addr?: string
  yellowstone_grpc: boolean
  rpc_port: number
  dynamic_port_range: string
  node_type?: string
  gossip_port?: number
  /** Client-specific CLI flags: "flag-name" -> "value" (empty string for bare flags) */
  validator_flags?: Record<string, string>
  geyser_plugin_configs?: string[]
  environment_vars?: Record<string, string>
  extra_args?: string[]
  restart_sec?: number
  log_rate_limit_disable?: boolean
  start_limit_disable?: boolean
  no_port_check?: boolean
}

export async function provisionNode(id: string, config: ProvisionRequest): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/provision`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  })
}

export async function fetchGrafanaSettings(): Promise<{ grafana_url: string }> {
  return api('/api/settings/grafana')
}

export async function saveGrafanaUrl(url: string): Promise<{ grafana_url: string }> {
  return api('/api/settings/grafana', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ grafana_url: url }),
  })
}

// ---------------------------------------------------------------------------
// Version / upgrade types and functions
// ---------------------------------------------------------------------------

export interface AvailableUpdate {
  version: string
  download_url: string
  sha256: string
  release_notes: string
}

export interface VersionInfo {
  current_version: string
  controller_update?: AvailableUpdate
  agent_update?: AvailableUpdate
  checked_at?: number
}

export async function fetchVersionInfo(): Promise<VersionInfo> {
  return api('/api/version')
}

export async function upgradeController(): Promise<{ ok: boolean; message: string }> {
  return api('/api/upgrade-controller', { method: 'POST' })
}

export async function upgradeAgent(id: string): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/upgrade-agent`, { method: 'POST' })
}
