import { useState, useEffect, useCallback } from 'react'
import { useNavigate } from 'react-router-dom'
import { fetchOverview, fetchNodes, fetchOnboardCommand } from '../api'
import type { FleetOverview, Node } from '../api'

const STATE_COLORS: Record<string, string> = {
  healthy: 'green',
  behind: 'yellow',
  offline: 'red',
  unhealthy: 'red',
  recovering: 'orange',
  registered: 'purple',
  provisioning: 'purple',
  starting_up: 'yellow',
}

function formatLastSeen(ts?: number): string {
  if (!ts) return '-'
  const ago = Math.floor(Date.now() / 1000 - ts)
  if (ago < 60) return `${ago}s ago`
  if (ago < 3600) return `${Math.floor(ago / 60)}m ago`
  if (ago < 86400) return `${Math.floor(ago / 3600)}h ago`
  return `${Math.floor(ago / 86400)}d ago`
}

function clusterLabel(cluster?: string): string {
  if (!cluster) return '-'
  if (cluster === 'mainnet-beta') return 'mainnet'
  return cluster
}

function Overview() {
  const navigate = useNavigate()
  const [overview, setOverview] = useState<FleetOverview | null>(null)
  const [nodes, setNodes] = useState<Node[]>([])
  const [onboardCmd, setOnboardCmd] = useState('')
  const [copied, setCopied] = useState(false)

  const refresh = useCallback(async () => {
    try {
      const [ov, ns] = await Promise.all([fetchOverview(), fetchNodes()])
      setOverview(ov)
      setNodes(ns)
    } catch {
      // API may not be available yet
    }
  }, [])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 10000)
    return () => clearInterval(interval)
  }, [refresh])

  useEffect(() => {
    fetchOnboardCommand()
      .then((res) => setOnboardCmd(res.command))
      .catch(() => {})
  }, [])

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(onboardCmd)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // clipboard may not be available
    }
  }

  const stateCount = (state: string): number => overview?.by_state[state] ?? 0

  return (
    <div>
      <div className="summary-cards">
        <div className="summary-card purple">
          <div className="label">Total Nodes</div>
          <div className="value">{overview?.total ?? 0}</div>
        </div>
        <div className="summary-card green">
          <div className="label">Healthy</div>
          <div className="value">{stateCount('healthy')}</div>
        </div>
        <div className="summary-card yellow">
          <div className="label">Behind</div>
          <div className="value">{stateCount('behind')}</div>
        </div>
        <div className="summary-card red">
          <div className="label">Offline</div>
          <div className="value">{stateCount('offline')}</div>
        </div>
        <div className="summary-card red">
          <div className="label">Unhealthy</div>
          <div className="value">{stateCount('unhealthy')}</div>
        </div>
      </div>

      <table className="node-table">
        <thead>
          <tr>
            <th>Host</th>
            <th>IP</th>
            <th>State</th>
            <th>Link</th>
            <th>Client</th>
            <th>Cluster</th>
            <th>Version</th>
            <th>Slots Behind</th>
            <th>Last Seen</th>
          </tr>
        </thead>
        <tbody>
          {nodes.map((node) => (
            <tr key={node.node_id} onClick={() => navigate(`/nodes/${node.node_id}`)}>
              <td>{node.hostname ?? node.live_status?.hostname ?? node.node_id}</td>
              <td>{node.ip_address && !node.ip_address.includes(':') ? node.ip_address : '-'}</td>
              <td>
                <span className={`badge ${STATE_COLORS[node.lifecycle_state] ? node.lifecycle_state : ''}`}>
                  {node.lifecycle_state}
                </span>
              </td>
              <td>
                <span className={`link-status ${node.live_status ? 'connected' : 'disconnected'}`}>
                  {node.live_status ? 'Connected' : 'Disconnected'}
                </span>
              </td>
              <td>{node.client ?? node.live_status?.client ?? '-'}</td>
              <td>
                {(node.cluster || node.live_status?.cluster) ? (
                  <span className={`cluster-badge ${node.cluster ?? node.live_status?.cluster ?? ''}`}>
                    {clusterLabel(node.cluster ?? node.live_status?.cluster)}
                  </span>
                ) : '-'}
              </td>
              <td>{node.live_status?.version ?? '-'}</td>
              <td>{node.live_status?.slots_behind ?? '-'}</td>
              <td>{formatLastSeen(node.last_seen_at)}</td>
            </tr>
          ))}
          {nodes.length === 0 && (
            <tr>
              <td colSpan={9} style={{ textAlign: 'center', color: 'var(--text-dim)', padding: '2rem' }}>
                No nodes connected. Use the command below to add one.
              </td>
            </tr>
          )}
        </tbody>
      </table>

      <div className="onboard-panel">
        <h3>Add a Node</h3>
        <p style={{ color: 'var(--text-dim)', fontSize: '0.8125rem', marginBottom: '0.75rem' }}>
          Run this command on any Linux machine to join it to your fleet:
        </p>
        <div className="onboard-command">
          <code>{onboardCmd || 'Loading...'}</code>
          <button className="btn primary" onClick={handleCopy}>
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      </div>
    </div>
  )
}

export default Overview
