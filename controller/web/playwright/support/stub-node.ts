import type { Page } from '@playwright/test'
import type { Node } from '../../src/api'

// Serves a fixed node from GET /api/nodes/:id so the UI can be exercised in a
// given lifecycle state without a live agent. Action POSTs are recorded and
// answered ok; everything else (logs, releases, auth) hits the real controller.

export function stubNode(overrides: Partial<Node> & { node_id: string }): Node {
  const now = Math.floor(Date.now() / 1000)
  return {
    lifecycle_state: 'healthy',
    role: 'rpc',
    client: 'surfpool',
    cluster: 'devnet',
    hostname: overrides.node_id,
    agent_version: '0.8.8',
    last_seen_at: now,
    registered_at: now - 3600,
    live_status: {
      state: 'healthy', local_slot: 100, reference_slot: 100, slots_behind: 0, healthy: true,
      restart_count: 0, crash_looping: false, version: '1.0.0', role: 'rpc', client: 'surfpool',
      cluster: 'devnet', cpu_usage_percent: 1, memory_used_bytes: 1, memory_total_bytes: 2,
      disk_used_bytes: 1, disk_total_bytes: 2, updated_at_unix_secs: now, state_duration_secs: 60,
      hostname: overrides.node_id,
    },
    ...overrides,
  }
}

export async function serveNode(page: Page, node: Node): Promise<{ actions: string[] }> {
  const actions: string[] = []
  const base = `/api/nodes/${encodeURIComponent(node.node_id)}`
  await page.route(`**${base}`, route => route.fulfill({ json: node }))
  await page.route(new RegExp(`${base}/(stop|restart|recover)$`), route => {
    actions.push(new URL(route.request().url()).pathname.split('/').pop()!)
    return route.fulfill({ json: { ok: true, message: 'ok' } })
  })
  return { actions }
}
