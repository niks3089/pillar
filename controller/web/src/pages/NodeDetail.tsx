import { useState, useEffect, useCallback, useRef } from 'react'
import { useParams, Link } from 'react-router-dom'
import { fetchNode, fetchNodeLogs, restartNode, recoverNode, deleteNode, provisionNode } from '../api'
import type { Node, LogEntry, ProvisionRequest } from '../api'

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`
}

function formatTimestamp(ms: number): string {
  return new Date(ms).toLocaleTimeString('en-US', { hour12: false })
}

function formatLastSeen(ts?: number): string {
  if (!ts) return '-'
  const ago = Math.floor(Date.now() / 1000 - ts)
  if (ago < 60) return `${ago}s ago`
  if (ago < 3600) return `${Math.floor(ago / 60)}m ago`
  return `${Math.floor(ago / 3600)}h ago`
}

const CLUSTER_ENTRYPOINTS: Record<string, string> = {
  'mainnet-beta': 'entrypoint.mainnet-beta.solana.com:8001\nentrypoint2.mainnet-beta.solana.com:8001\nentrypoint3.mainnet-beta.solana.com:8001',
  'testnet': 'entrypoint.testnet.solana.com:8001\nentrypoint2.testnet.solana.com:8001\nentrypoint3.testnet.solana.com:8001',
  'devnet': 'entrypoint.devnet.solana.com:8001',
}

const CLUSTER_KNOWN_VALIDATORS: Record<string, string> = {
  'mainnet-beta': '7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2\nGdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ\nDE1bawNcRJB9rVm3buyMVfr8mBEoyyu73NBovf2oXJsJ\nCakcnaRDHka2gXyfbEd2d3xsvkJkqsLw2akB3zsN1D2S',
  'testnet': '5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on\ndDzy5SR3AXdYWVqbDEkVFdvSPCtS9ihF5kJkHCtXoFs\nFS9MmFpFd1iMSSwzDYnqLPhWkoXKhJGBRCq1SFRsqFB\neoKpUABi59aT4with2BRcnKHr6MAxfY53VNa1yoV3Cy',
  'devnet': '',
}

function NodeDetail() {
  const { id } = useParams<{ id: string }>()
  const [node, setNode] = useState<Node | null>(null)
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [error, setError] = useState('')
  const [logFilter, setLogFilter] = useState<string>('all')
  const [sseConnected, setSseConnected] = useState(false)
  const logContainerRef = useRef<HTMLDivElement>(null)
  const eventSourceRef = useRef<EventSource | null>(null)

  // Provision form state
  const [provClient, setProvClient] = useState('agave')
  const [provVersion, setProvVersion] = useState('')
  const [provCluster, setProvCluster] = useState('mainnet-beta')
  const [provLedgerPath, setProvLedgerPath] = useState('/mnt/ledger')
  const [provSnapshotPath, setProvSnapshotPath] = useState('/mnt/snapshots')
  const [provAccountsPath, setProvAccountsPath] = useState('/mnt/accounts')
  const [provIdentityPath, setProvIdentityPath] = useState('/home/sol/validator-keypair.json')
  const [provVotePath, setProvVotePath] = useState('')
  const [provEntrypoints, setProvEntrypoints] = useState(CLUSTER_ENTRYPOINTS['mainnet-beta'])
  const [provKnownValidators, setProvKnownValidators] = useState(CLUSTER_KNOWN_VALIDATORS['mainnet-beta'])
  const [provDownloadUrl, setProvDownloadUrl] = useState('')
  const [provSha256, setProvSha256] = useState('')
  const [provJitoMev, setProvJitoMev] = useState(false)
  const [provJitoBlockEngineUrl, setProvJitoBlockEngineUrl] = useState('')
  const [provYellowstoneGrpc, setProvYellowstoneGrpc] = useState(false)
  const [provRpcPort, setProvRpcPort] = useState('8899')
  const [provDynamicPortRange, setProvDynamicPortRange] = useState('8000-8020')
  const [provSubmitting, setProvSubmitting] = useState(false)

  const refresh = useCallback(async () => {
    if (!id) return
    try {
      const n = await fetchNode(id)
      setNode(n)
      setError('')
    } catch (err) {
      setError(String(err))
    }
  }, [id])

  // Load initial logs
  useEffect(() => {
    if (!id) return
    fetchNodeLogs(id, { limit: 200 })
      .then(setLogs)
      .catch(() => {})
  }, [id])

  // SSE for live logs
  useEffect(() => {
    if (!id) return
    const es = new EventSource(`/api/nodes/${encodeURIComponent(id)}/logs/stream`)
    eventSourceRef.current = es

    es.onopen = () => setSseConnected(true)
    es.onerror = () => setSseConnected(false)

    es.onmessage = (event) => {
      try {
        const entry: LogEntry = JSON.parse(event.data)
        setLogs((prev) => [...prev.slice(-999), entry])
      } catch {
        // ignore parse errors
      }
    }

    return () => {
      es.close()
      eventSourceRef.current = null
      setSseConnected(false)
    }
  }, [id])

  // Auto-scroll logs
  useEffect(() => {
    const el = logContainerRef.current
    if (el) {
      el.scrollTop = el.scrollHeight
    }
  }, [logs])

  // Refresh node data
  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 10000)
    return () => clearInterval(interval)
  }, [refresh])

  const handleRestart = async () => {
    if (!id || !confirm('Restart this node?')) return
    await restartNode(id)
    refresh()
  }

  const handleRecover = async () => {
    if (!id || !confirm('Trigger snapshot recovery on this node? This will stop the validator and re-download a snapshot.')) return
    await recoverNode(id)
    refresh()
  }

  const handleDelete = async () => {
    if (!id || !confirm('Remove this node from the fleet? This cannot be undone.')) return
    await deleteNode(id)
    window.location.href = '/'
  }

  const handleClusterChange = (cluster: string) => {
    setProvCluster(cluster)
    setProvEntrypoints(CLUSTER_ENTRYPOINTS[cluster] || '')
    setProvKnownValidators(CLUSTER_KNOWN_VALIDATORS[cluster] || '')
  }

  const handleProvision = async () => {
    if (!id) return
    if (!provClient || !provVersion || !provCluster) {
      alert('Client, version, and cluster are required.')
      return
    }
    if (!confirm(`Install ${provClient} ${provVersion} on ${provCluster} for this node?`)) return

    setProvSubmitting(true)
    try {
      const config: ProvisionRequest = {
        client: provClient,
        version: provVersion,
        cluster: provCluster,
        identity_keypair_path: provIdentityPath,
        vote_account_keypair_path: provVotePath,
        ledger_path: provLedgerPath,
        snapshot_path: provSnapshotPath,
        accounts_path: provAccountsPath,
        entrypoints: provEntrypoints.split('\n').map(s => s.trim()).filter(Boolean),
        known_validators: provKnownValidators.split('\n').map(s => s.trim()).filter(Boolean),
        download_url: provDownloadUrl,
        sha256: provSha256,
        jito_mev: provJitoMev,
        jito_block_engine_url: provJitoBlockEngineUrl,
        yellowstone_grpc: provYellowstoneGrpc,
        rpc_port: parseInt(provRpcPort) || 8899,
        dynamic_port_range: provDynamicPortRange,
      }
      const result = await provisionNode(id, config)
      if (result.ok) {
        alert('Provision command sent successfully.')
        refresh()
      } else {
        alert(`Failed: ${result.message}`)
      }
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setProvSubmitting(false)
    }
  }

  if (error && !node) {
    return (
      <div>
        <Link to="/" className="back-link">&larr; Back to Overview</Link>
        <p style={{ color: 'var(--red)' }}>Error loading node: {error}</p>
      </div>
    )
  }

  if (!node) {
    return (
      <div>
        <Link to="/" className="back-link">&larr; Back to Overview</Link>
        <p style={{ color: 'var(--text-dim)' }}>Loading...</p>
      </div>
    )
  }

  const s = node.live_status

  return (
    <div>
      <Link to="/" className="back-link">&larr; Back to Overview</Link>

      <div className="node-header">
        <h1>{node.node_id}</h1>
        <span className={`badge ${node.lifecycle_state}`}>{node.lifecycle_state}</span>
        <span className={`link-status ${node.live_status ? 'connected' : 'disconnected'}`}>
          {node.live_status ? 'Link Connected' : 'Link Disconnected'}
        </span>
        {node.hostname && <span className="meta">{node.hostname}</span>}
        <span className="meta">Last seen: {formatLastSeen(node.last_seen_at)}</span>
      </div>

      <div className="metrics-grid">
        <div className="metric-card">
          <div className="label">Slots Behind</div>
          <div className="value">{s?.slots_behind ?? '-'}</div>
        </div>
        <div className="metric-card">
          <div className="label">CPU</div>
          <div className="value">{s ? `${s.cpu_usage_percent.toFixed(1)}%` : '-'}</div>
        </div>
        <div className="metric-card">
          <div className="label">Memory</div>
          <div className="value">
            {s ? `${formatBytes(s.memory_used_bytes)} / ${formatBytes(s.memory_total_bytes)}` : '-'}
          </div>
        </div>
        <div className="metric-card">
          <div className="label">Disk</div>
          <div className="value">
            {s ? `${formatBytes(s.disk_used_bytes)} / ${formatBytes(s.disk_total_bytes)}` : '-'}
          </div>
        </div>
        <div className="metric-card">
          <div className="label">Restarts</div>
          <div className="value">{s?.restart_count ?? '-'}</div>
        </div>
        <div className="metric-card">
          <div className="label">Version</div>
          <div className="value" style={{ fontSize: '1rem' }}>{s?.version ?? '-'}</div>
        </div>
      </div>

      <div className="actions">
        <button className="btn primary" onClick={handleRestart}>Restart</button>
        <button className="btn" onClick={handleRecover}>Recover</button>
        <button className="btn danger" onClick={handleDelete}>Delete</button>
      </div>

      <div className="provision-panel">
        <h2>Setup Validator</h2>
        <div className="form-grid">
          <div className="form-group">
            <label>Client</label>
            <select value={provClient} onChange={e => setProvClient(e.target.value)}>
              <option value="agave">Agave</option>
              <option value="jito">Jito</option>
              <option value="firedancer">Firedancer</option>
              <option value="frankendancer">Frankendancer</option>
            </select>
          </div>
          <div className="form-group">
            <label>Cluster</label>
            <select value={provCluster} onChange={e => handleClusterChange(e.target.value)}>
              <option value="mainnet-beta">mainnet-beta</option>
              <option value="testnet">testnet</option>
              <option value="devnet">devnet</option>
            </select>
          </div>
          <div className="form-group">
            <label>Version</label>
            <input type="text" value={provVersion} onChange={e => setProvVersion(e.target.value)} placeholder="e.g. 2.1.6" />
          </div>
          <div className="form-group">
            <label>Ledger Path</label>
            <input type="text" value={provLedgerPath} onChange={e => setProvLedgerPath(e.target.value)} />
          </div>
          <div className="form-group">
            <label>Snapshot Path</label>
            <input type="text" value={provSnapshotPath} onChange={e => setProvSnapshotPath(e.target.value)} />
          </div>
          <div className="form-group">
            <label>Accounts Path</label>
            <input type="text" value={provAccountsPath} onChange={e => setProvAccountsPath(e.target.value)} />
          </div>
          <div className="form-group">
            <label>Identity Keypair Path</label>
            <input type="text" value={provIdentityPath} onChange={e => setProvIdentityPath(e.target.value)} />
          </div>
          <div className="form-group">
            <label>Vote Account Keypair Path</label>
            <input type="text" value={provVotePath} onChange={e => setProvVotePath(e.target.value)} placeholder="/home/sol/vote-account-keypair.json" />
          </div>
          <div className="form-group">
            <label>RPC Port</label>
            <input type="text" value={provRpcPort} onChange={e => setProvRpcPort(e.target.value)} placeholder="8899" />
          </div>
          <div className="form-group">
            <label>Dynamic Port Range</label>
            <input type="text" value={provDynamicPortRange} onChange={e => setProvDynamicPortRange(e.target.value)} placeholder="8000-8020" />
          </div>
        </div>

        <div className="form-group" style={{ marginTop: '1rem' }}>
          <label>Entrypoints</label>
          <textarea rows={3} value={provEntrypoints} onChange={e => setProvEntrypoints(e.target.value)} placeholder="One per line" />
        </div>
        <div className="form-group">
          <label>Known Validators</label>
          <textarea rows={4} value={provKnownValidators} onChange={e => setProvKnownValidators(e.target.value)} placeholder="One pubkey per line" />
        </div>

        <div className="form-grid" style={{ marginTop: '1rem' }}>
          <div className="form-group">
            <label>Download URL</label>
            <input type="text" value={provDownloadUrl} onChange={e => setProvDownloadUrl(e.target.value)} placeholder="https://..." />
          </div>
          <div className="form-group">
            <label>SHA256</label>
            <input type="text" value={provSha256} onChange={e => setProvSha256(e.target.value)} placeholder="Expected hash of binary" />
          </div>
        </div>

        <div className="form-group" style={{ marginTop: '1rem' }}>
          <label className="checkbox-label">
            <input type="checkbox" checked={provJitoMev} onChange={e => setProvJitoMev(e.target.checked)} />
            Jito MEV
          </label>
          {provJitoMev && (
            <div style={{ marginTop: '0.5rem' }}>
              <label>Block Engine URL</label>
              <input type="text" value={provJitoBlockEngineUrl} onChange={e => setProvJitoBlockEngineUrl(e.target.value)} placeholder="https://..." />
            </div>
          )}
        </div>
        <div className="form-group">
          <label className="checkbox-label">
            <input type="checkbox" checked={provYellowstoneGrpc} onChange={e => setProvYellowstoneGrpc(e.target.checked)} />
            Yellowstone gRPC
          </label>
        </div>

        <button className="btn primary" onClick={handleProvision} disabled={provSubmitting} style={{ marginTop: '1rem' }}>
          {provSubmitting ? 'Sending...' : 'Install Validator'}
        </button>
      </div>

      <div className="logs-section">
        <div className="logs-header">
          <h2>Logs</h2>
          <span className={`live-indicator ${sseConnected ? 'connected' : ''}`}>
            {sseConnected ? 'Live' : 'Disconnected'}
          </span>
        </div>
        <div className="log-tabs">
          {['all', 'controller', 'validator', 'operator', 'link'].map(tab => (
            <button
              key={tab}
              className={`log-tab ${logFilter === tab ? 'active' : ''}`}
              onClick={() => setLogFilter(tab)}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
        <div className="log-container" ref={logContainerRef}>
          {logs.filter(e => logFilter === 'all' || e.service === logFilter).length === 0 && (
            <div style={{ color: 'var(--text-dim)', padding: '1rem', textAlign: 'center' }}>
              No logs available
            </div>
          )}
          {logs
            .filter(e => logFilter === 'all' || e.service === logFilter)
            .map((entry) => (
            <div key={entry.id} className={`log-entry ${entry.level}`}>
              <span className="timestamp">{formatTimestamp(entry.timestamp_ms)}</span>
              <span className="service">{entry.service}</span>
              <span className={`level ${entry.level}`}>{entry.level.toUpperCase().padEnd(5)}</span>
              <span className="message">{entry.message}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}

export default NodeDetail
