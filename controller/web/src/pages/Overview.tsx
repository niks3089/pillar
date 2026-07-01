import { useState, useEffect, useCallback } from 'react'
import { useNavigate } from 'react-router-dom'
import { fetchOverview, fetchNodes, fetchOnboardCommand } from '../api'
import type { FleetOverview, Node } from '../api'

const STATE_BADGE_CLASSES: Record<string, string> = {
  healthy: 'bg-green-500/10 text-green-400 border-green-500/20',
  behind: 'bg-yellow-500/10 text-yellow-400 border-yellow-500/20',
  offline: 'bg-red-500/10 text-red-400 border-red-500/20',
  unhealthy: 'bg-red-500/10 text-red-400 border-red-500/20',
  recovering: 'bg-orange-500/10 text-orange-400 border-orange-500/20',
  registered: 'bg-purple-500/10 text-purple-400 border-purple-500/20',
  provisioning: 'bg-purple-500/10 text-purple-400 border-purple-500/20',
  starting_up: 'bg-yellow-500/10 text-yellow-400 border-yellow-500/20',
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
    <div className="flex flex-col gap-8">
      {/* Summary Cards */}
      <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Total Validators</div>
          <div className="text-3xl font-semibold text-purple-400">{overview?.total ?? 0}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Healthy</div>
          <div className="text-3xl font-semibold text-green-400">{stateCount('healthy')}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Behind</div>
          <div className="text-3xl font-semibold text-yellow-400">{stateCount('behind')}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Offline</div>
          <div className="text-3xl font-semibold text-red-400">{stateCount('offline')}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Unhealthy</div>
          <div className="text-3xl font-semibold text-red-400">{stateCount('unhealthy')}</div>
        </div>
      </div>

      {/* Validators Table */}
      <div className="bg-[#15131f] border border-white/10 rounded-xl overflow-x-auto shadow-sm">
        <table className="w-full text-left border-collapse whitespace-nowrap">
          <thead>
            <tr className="bg-white/[0.02] border-b border-white/10">
              <th className="px-6 py-4 text-xs font-semibold text-zinc-400 uppercase tracking-wider">Node</th>
              <th className="px-6 py-4 text-xs font-semibold text-zinc-400 uppercase tracking-wider">Status</th>
              <th className="px-6 py-4 text-xs font-semibold text-zinc-400 uppercase tracking-wider">Software</th>
              <th className="px-6 py-4 text-xs font-semibold text-zinc-400 uppercase tracking-wider text-right">Slots Behind</th>
              <th className="px-6 py-4 text-xs font-semibold text-zinc-400 uppercase tracking-wider text-right">Last Seen</th>
              <th className="px-6 py-4 text-xs font-semibold text-zinc-400 uppercase tracking-wider text-right">Actions</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-white/5">
            {nodes.map((node) => {
              const client = node.client ?? node.live_status?.client ?? '-';
              const version = node.live_status?.version ?? '-';
              const cluster = node.cluster ?? node.live_status?.cluster;
              
              return (
              <tr 
                key={node.node_id} 
                onClick={() => navigate(`/nodes/${node.node_id}`)}
                className="hover:bg-white/[0.03] transition-colors cursor-pointer group"
              >
                {/* Node Identity & Connection */}
                <td className="px-6 py-4">
                  <div className="flex items-center gap-2 mb-1">
                    <div className={`w-2 h-2 rounded-full shrink-0 ${node.live_status ? 'bg-green-500 shadow-[0_0_8px_rgba(34,197,94,0.4)]' : 'bg-red-500'}`} title={node.live_status ? 'Connected' : 'Disconnected'}></div>
                    <div className="text-sm font-medium text-zinc-200">{node.node_id}</div>
                  </div>
                  <div className="text-xs text-zinc-500 font-mono ml-4 flex gap-2 items-center">
                    {node.hostname && node.hostname !== node.node_id ? <span>{node.hostname}</span> : null}
                    {node.ip_address && !node.ip_address.includes(':') && (
                      <>
                        {node.hostname && node.hostname !== node.node_id && <span className="text-zinc-700">•</span>}
                        <span>{node.ip_address}</span>
                      </>
                    )}
                  </div>
                </td>
                
                {/* Status (Lifecycle) */}
                <td className="px-6 py-4">
                  <span className={`inline-flex items-center px-2 py-0.5 text-[11px] font-medium uppercase tracking-wider rounded border ${STATE_BADGE_CLASSES[node.lifecycle_state] || 'bg-zinc-500/10 text-zinc-400 border-zinc-500/20'}`}>
                    {node.lifecycle_state}
                  </span>
                </td>
                
                {/* Software & Cluster */}
                <td className="px-6 py-4">
                  <div className="text-sm font-medium text-zinc-300 capitalize flex items-center gap-2 mb-1">
                    {client} <span className="text-zinc-500 font-mono text-xs lowercase">v{version}</span>
                  </div>
                  {cluster ? (
                    <span className="inline-flex items-center px-2 py-0.5 text-[10px] font-medium uppercase tracking-wider bg-zinc-800 text-zinc-400 rounded border border-zinc-700">
                      {clusterLabel(cluster)}
                    </span>
                  ) : <span className="text-zinc-600 text-xs">-</span>}
                </td>
                
                {/* Slots Behind */}
                <td className="px-6 py-4 text-sm text-zinc-300 text-right font-mono">
                  {node.live_status?.slots_behind !== undefined ? (
                    <span className={node.live_status.slots_behind > 10 ? "text-yellow-400" : ""}>
                      {node.live_status.slots_behind.toLocaleString()}
                    </span>
                  ) : (
                    <span className="text-zinc-600">-</span>
                  )}
                </td>
                
                {/* Last Seen */}
                <td className="px-6 py-4 text-sm text-zinc-400 text-right">
                  {formatLastSeen(node.last_seen_at)}
                </td>
                
                {/* Actions */}
                <td className="px-6 py-4 text-sm text-right">
                  <a
                    onClick={e => e.stopPropagation()}
                    href={`/grafana/d/pillar-node-detail/pillar-node-detail?orgId=1&from=now-1h&to=now&timezone=browser&var-datasource=pillar-prometheus&var-node_id=${encodeURIComponent(node.node_id)}&refresh=30s`}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-purple-400 hover:text-purple-300 font-medium inline-flex items-center gap-1 group-hover:underline"
                  >
                    Metrics <span className="text-[10px]">↗</span>
                  </a>
                </td>
              </tr>
            )})}
            {nodes.length === 0 && (
              <tr>
                <td colSpan={6} className="px-6 py-12 text-center text-sm text-zinc-500">
                  No validators connected. Use the command below to add one.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* Onboarding Panel */}
      <div className="bg-[#15131f] border border-purple-500/20 rounded-xl p-6 shadow-[0_0_20px_rgba(153,69,255,0.05)]">
        <h3 className="text-lg font-semibold text-zinc-100 mb-1">Add a Validator</h3>
        <p className="text-sm text-zinc-400 mb-4">Run this command on the validator host to add it to your fleet:</p>
        <div className="flex items-center gap-3 bg-black/40 border border-white/10 rounded-lg p-3">
          <code className="flex-1 text-sm text-green-400 font-mono overflow-x-auto whitespace-nowrap scrollbar-hide">
            {onboardCmd || 'Loading...'}
          </code>
          <button 
            className="shrink-0 px-4 py-2 text-sm font-medium text-white bg-white/10 hover:bg-white/20 rounded-md transition-colors" 
            onClick={handleCopy}
          >
            {copied ? 'Copied!' : 'Copy'}
          </button>
        </div>
      </div>
    </div>
  )
}

export default Overview
