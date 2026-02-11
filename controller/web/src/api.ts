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
  operator_version?: string
  link_version?: string
  last_seen_at?: number
  registered_at?: number
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

export async function fetchNodeHistory(id: string, limit?: number): Promise<unknown[]> {
  const params = limit ? `?limit=${limit}` : ''
  return api(`/api/nodes/${encodeURIComponent(id)}/history${params}`)
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

export async function restartNode(id: string): Promise<void> {
  await fetch(`/api/nodes/${encodeURIComponent(id)}/restart`, { method: 'POST' })
}

export async function recoverNode(id: string): Promise<void> {
  await fetch(`/api/nodes/${encodeURIComponent(id)}/recover`, { method: 'POST' })
}

export async function deleteNode(id: string): Promise<void> {
  await fetch(`/api/nodes/${encodeURIComponent(id)}`, { method: 'DELETE' })
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
  yellowstone_grpc: boolean
  rpc_port: number
  dynamic_port_range: string
}

export async function provisionNode(id: string, config: ProvisionRequest): Promise<{ ok: boolean; message: string }> {
  return api(`/api/nodes/${encodeURIComponent(id)}/provision`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(config),
  })
}
