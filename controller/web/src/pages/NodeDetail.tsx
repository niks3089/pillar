import { useState, useEffect, useCallback, useRef } from 'react'
import { useParams, Link } from 'react-router-dom'
import { fetchNode, fetchNodeLogs, restartNode, recoverNode, deleteNode, stopNode, cancelDeployment, provisionNode } from '../api'
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
  'mainnet-beta': 'entrypoint.mainnet-beta.solana.com:8001\nentrypoint2.mainnet-beta.solana.com:8001\nentrypoint3.mainnet-beta.solana.com:8001\nentrypoint4.mainnet-beta.solana.com:8001\nentrypoint5.mainnet-beta.solana.com:8001',
  'testnet': 'entrypoint.testnet.solana.com:8001\nentrypoint2.testnet.solana.com:8001\nentrypoint3.testnet.solana.com:8001\nentrypoint4.testnet.solana.com:8001\nentrypoint5.testnet.solana.com:8001',
  'devnet': 'entrypoint.devnet.solana.com:8001\nentrypoint2.devnet.solana.com:8001\nentrypoint3.devnet.solana.com:8001\nentrypoint4.devnet.solana.com:8001\nentrypoint5.devnet.solana.com:8001',
}

const CLUSTER_KNOWN_VALIDATORS: Record<string, string> = {
  'mainnet-beta': '7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2\nGdnSyH3YtwcxFvQrVVJMm1JhTS4QVX7MFsX56uJLUfiZ\nDE1bawNcRJB9rVm3buyMVfr8mBEoyyu73NBovf2oXJsJ\nCakcnaRDHka2gXyfbEd2d3xsvkJkqsLw2akB3zsN1D2S',
  'testnet': '5D1fNXzvv5NjV1ysLjirC4WY92RNsVH18vjmcszZd8on\ndDzy5SR3AXdYWVqbDEkVFdvSPCtS9ihF5kJkHCtXoFs\nFS9MmFpFd1iMSSwzDYnqLPhWkoXKhJGBRCq1SFRsqFB\neoKpUABi59aT4with2BRcnKHr6MAxfY53VNa1yoV3Cy',
  'devnet': '',
}

const CLUSTER_GENESIS_HASH: Record<string, string> = {
  'mainnet-beta': '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d',
  'testnet': '4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY',
  'devnet': 'EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG',
}

// Flags presets per node type. Format: "flag-name" (bare) or "flag-name=value".
// These are the defaults the UI fills in; users can edit freely.
function buildPreset(cluster: string, nodeType: string): string {
  const genesis = CLUSTER_GENESIS_HASH[cluster] || ''
  const common = [
    'rpc-bind-address=0.0.0.0',
    'no-port-check',
    'wal-recovery-mode=skip_any_corrupted_record',
    'limit-ledger-size',
  ]
  if (genesis) common.push(`expected-genesis-hash=${genesis}`)

  switch (nodeType) {
    case 'rpc':
      return [...common, 'no-voting', 'private-rpc', 'full-rpc-api', 'enable-rpc-transaction-history', 'no-skip-initial-accounts-db-clean'].join('\n')
    case 'archival':
      return [...common.filter(f => f !== 'limit-ledger-size'), 'limit-ledger-size=500000000', 'no-voting', 'private-rpc', 'full-rpc-api', 'enable-rpc-transaction-history', 'enable-extended-tx-metadata-storage', 'no-skip-initial-accounts-db-clean'].join('\n')
    default: // validator
      return common.join('\n')
  }
}

// Default presets for the initial form state (validator on mainnet)
const VALIDATOR_PRESETS: Record<string, string> = {
  'mainnet-beta': buildPreset('mainnet-beta', 'validator'),
  'testnet': buildPreset('testnet', 'validator'),
  'devnet': buildPreset('devnet', 'validator'),
}

/** Parse "flag-name" or "flag-name=value" lines into a Record */
function parseFlags(text: string): Record<string, string> {
  const flags: Record<string, string> = {}
  text.split('\n').map(s => s.trim()).filter(Boolean).forEach(line => {
    const eq = line.indexOf('=')
    if (eq > 0) {
      flags[line.slice(0, eq)] = line.slice(eq + 1)
    } else {
      flags[line] = ''
    }
  })
  return flags
}

/** Parse "KEY=VALUE" lines into a Record */
function parseEnvVars(text: string): Record<string, string> {
  const vars: Record<string, string> = {}
  text.split('\n').map(s => s.trim()).filter(Boolean).forEach(line => {
    const eq = line.indexOf('=')
    if (eq > 0) vars[line.slice(0, eq)] = line.slice(eq + 1)
  })
  return vars
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
  const [provNodeType, setProvNodeType] = useState('validator')
  const [provGossipPort, setProvGossipPort] = useState('8001')
  // Validator flags as "flag-name" or "flag-name=value" lines (one per line)
  const [provValidatorFlags, setProvValidatorFlags] = useState(VALIDATOR_PRESETS['mainnet-beta'])
  const [provGeyserPluginConfigs, setProvGeyserPluginConfigs] = useState('')
  const [provEnvironmentVars, setProvEnvironmentVars] = useState('')
  const [provExtraArgs, setProvExtraArgs] = useState('')
  const [provRestartSec, setProvRestartSec] = useState('1')
  const [provLogRateLimitDisable, setProvLogRateLimitDisable] = useState(true)
  const [provStartLimitDisable, setProvStartLimitDisable] = useState(true)
  const [showAdvanced, setShowAdvanced] = useState(false)

  // Derived: check if no-voting is present in the flags textarea
  const noVotingActive = provValidatorFlags.split('\n').some(l => l.trim() === 'no-voting')

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

  const handleStop = async () => {
    if (!id || !confirm('Stop the validator on this node? It will not restart automatically.')) return
    try {
      const result = await stopNode(id)
      if (result.ok) {
        refresh()
      } else {
        alert(`Failed: ${result.message}`)
      }
    } catch (err) {
      alert(`Error: ${err}`)
    }
  }

  const handleCancel = async () => {
    if (!id || !confirm('Cancel the in-progress deployment? The validator will be stopped.')) return
    try {
      const result = await cancelDeployment(id)
      if (result.ok) {
        refresh()
      } else {
        alert(`Failed: ${result.message}`)
      }
    } catch (err) {
      alert(`Error: ${err}`)
    }
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
    setProvValidatorFlags(buildPreset(cluster, provNodeType))
  }

  const handleNodeTypeChange = (nodeType: string) => {
    setProvNodeType(nodeType)
    setProvValidatorFlags(buildPreset(provCluster, nodeType))
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
      const validatorFlags = parseFlags(provValidatorFlags)
      const envVars = parseEnvVars(provEnvironmentVars)
      const isNoVoting = 'no-voting' in validatorFlags
      const geyserConfigs = provGeyserPluginConfigs.split('\n').map(s => s.trim()).filter(Boolean)
      const extraArgsList = provExtraArgs.split('\n').map(s => s.trim()).filter(Boolean)

      const config: ProvisionRequest = {
        client: provClient,
        version: provVersion,
        cluster: provCluster,
        identity_keypair_path: provIdentityPath,
        vote_account_keypair_path: isNoVoting ? '' : provVotePath,
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
        node_type: provNodeType,
        gossip_port: parseInt(provGossipPort) || 8001,
        validator_flags: Object.keys(validatorFlags).length > 0 ? validatorFlags : undefined,
        geyser_plugin_configs: geyserConfigs.length > 0 ? geyserConfigs : undefined,
        environment_vars: Object.keys(envVars).length > 0 ? envVars : undefined,
        extra_args: extraArgsList.length > 0 ? extraArgsList : undefined,
        restart_sec: parseInt(provRestartSec) || 1,
        log_rate_limit_disable: provLogRateLimitDisable,
        start_limit_disable: provStartLimitDisable,
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
        <button className="btn" onClick={handleStop}>Stop</button>
        {(node.lifecycle_state === 'provisioning' || node.lifecycle_state === 'starting_up') && (
          <button className="btn danger" onClick={handleCancel}>Cancel Deployment</button>
        )}
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
            <label>Node Type</label>
            <select value={provNodeType} onChange={e => handleNodeTypeChange(e.target.value)}>
              <option value="validator">Validator</option>
              <option value="rpc">RPC Node</option>
              <option value="archival">Archival RPC</option>
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
          {!noVotingActive && (
            <div className="form-group">
              <label>Vote Account Keypair Path</label>
              <input type="text" value={provVotePath} onChange={e => setProvVotePath(e.target.value)} placeholder="/home/sol/vote-account-keypair.json" />
            </div>
          )}
          <div className="form-group">
            <label>RPC Port</label>
            <input type="text" value={provRpcPort} onChange={e => setProvRpcPort(e.target.value)} placeholder="8899" />
          </div>
          <div className="form-group">
            <label>Gossip Port</label>
            <input type="text" value={provGossipPort} onChange={e => setProvGossipPort(e.target.value)} placeholder="8001" />
          </div>
          <div className="form-group">
            <label>Dynamic Port Range</label>
            <input type="text" value={provDynamicPortRange} onChange={e => setProvDynamicPortRange(e.target.value)} placeholder="8000-8020" />
          </div>
        </div>

        <div className="form-group" style={{ marginTop: '1rem' }}>
          <label>Entrypoints</label>
          <textarea rows={5} value={provEntrypoints} onChange={e => setProvEntrypoints(e.target.value)} placeholder="One per line" />
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

        {/* Advanced Settings */}
        <div style={{ marginTop: '1.5rem', borderTop: '1px solid var(--border)', paddingTop: '1rem' }}>
          <button
            className="btn"
            onClick={() => setShowAdvanced(!showAdvanced)}
            style={{ marginBottom: '1rem' }}
          >
            {showAdvanced ? 'Hide' : 'Show'} Advanced Settings
          </button>

          {showAdvanced && (
            <div>
              <h3 style={{ margin: '0.5rem 0' }}>Validator Flags</h3>
              <div className="form-group">
                <label>One flag per line: <code>flag-name</code> (bare) or <code>flag-name=value</code>. Pre-populated from cluster/node-type presets.</label>
                <textarea
                  rows={12}
                  value={provValidatorFlags}
                  onChange={e => setProvValidatorFlags(e.target.value)}
                  placeholder="no-port-check&#10;limit-ledger-size&#10;rpc-bind-address=0.0.0.0&#10;expected-genesis-hash=5eykt4..."
                  style={{ fontFamily: 'monospace' }}
                />
              </div>

              {provJitoMev && (
                <div style={{ margin: '0.5rem 0 1rem', padding: '0.75rem', background: 'var(--surface)', borderRadius: '6px', fontSize: '0.85rem', color: 'var(--text-dim)' }}>
                  Jito MEV is enabled. Add these flags above if needed: <code>tip-payment-pubkey</code>, <code>tip-distribution-pubkey</code>, <code>commission-bps</code>
                </div>
              )}

              <h3 style={{ margin: '1rem 0 0.5rem' }}>Geyser Plugins</h3>
              <div className="form-group">
                <label>Plugin Config Paths (one per line)</label>
                <textarea rows={2} value={provGeyserPluginConfigs} onChange={e => setProvGeyserPluginConfigs(e.target.value)} placeholder="/etc/pillar/custom-geyser.json" />
              </div>

              <h3 style={{ margin: '1rem 0 0.5rem' }}>Systemd Tuning</h3>
              <div className="form-grid">
                <div className="form-group">
                  <label>RestartSec</label>
                  <input type="text" value={provRestartSec} onChange={e => setProvRestartSec(e.target.value)} placeholder="1" />
                </div>
                <div className="form-group">
                  <label className="checkbox-label">
                    <input type="checkbox" checked={provLogRateLimitDisable} onChange={e => setProvLogRateLimitDisable(e.target.checked)} />
                    LogRateLimitIntervalSec=0
                  </label>
                </div>
                <div className="form-group">
                  <label className="checkbox-label">
                    <input type="checkbox" checked={provStartLimitDisable} onChange={e => setProvStartLimitDisable(e.target.checked)} />
                    StartLimitIntervalSec=0
                  </label>
                </div>
              </div>

              <h3 style={{ margin: '1rem 0 0.5rem' }}>Environment Variables</h3>
              <div className="form-group">
                <label>KEY=VALUE (one per line)</label>
                <textarea rows={3} value={provEnvironmentVars} onChange={e => setProvEnvironmentVars(e.target.value)} placeholder="SOLANA_METRICS_CONFIG=host=https://metrics.solana.com:8086,db=mainnet-beta" />
              </div>

              <h3 style={{ margin: '1rem 0 0.5rem' }}>Extra CLI Arguments</h3>
              <div className="form-group">
                <label>Additional flags (one per line)</label>
                <textarea rows={3} value={provExtraArgs} onChange={e => setProvExtraArgs(e.target.value)} placeholder="--custom-flag value" />
              </div>
            </div>
          )}
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
          {['all', 'controller', 'validator', 'agent'].map(tab => (
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
