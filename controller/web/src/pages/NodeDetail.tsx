import { useState, useEffect, useCallback, useRef } from 'react'
import { useParams, Link } from 'react-router-dom'
import { fetchNode, fetchNodeLogs, restartNode, recoverNode, deleteNode, stopNode, cancelDeployment, provisionNode, fetchVersionInfo, upgradeAgent } from '../api'
import type { Node, LogEntry, ProvisionRequest, VersionInfo } from '../api'

const STATE_BADGE_CLASSES: Record<string, string> = {
  unprovisioned: 'bg-zinc-500/10 text-zinc-400 border-zinc-500/20',
  provisioning: 'bg-yellow-500/10 text-yellow-500 border-yellow-500/20',
  starting_up: 'bg-blue-500/10 text-blue-400 border-blue-500/20',
  healthy: 'bg-green-500/10 text-green-400 border-green-500/20',
  unhealthy: 'bg-red-500/10 text-red-400 border-red-500/20',
  shutting_down: 'bg-orange-500/10 text-orange-400 border-orange-500/20',
  terminated: 'bg-zinc-800 text-zinc-500 border-zinc-700'
}
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`
}

function formatTimestamp(ms: number): string {
  if (!ms || isNaN(ms)) return '--:--:--'
  const d = new Date(ms)
  const month = d.toLocaleString('en-US', { month: 'short' })
  const day = String(d.getDate()).padStart(2, '0')
  const time = d.toLocaleTimeString('en-US', { hour12: false })
  const millis = String(d.getMilliseconds()).padStart(3, '0')
  return `${month} ${day} ${time}.${millis}`
}

function formatLastSeen(ts?: number): string {
  if (!ts) return '-'
  const ago = Math.floor(Date.now() / 1000 - ts)
  if (ago < 60) return `${ago}s ago`
  if (ago < 3600) return `${Math.floor(ago / 60)}m ago`
  return `${Math.floor(ago / 3600)}h ago`
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  const days = Math.floor(secs / 86400)
  const hours = Math.floor((secs % 86400) / 3600)
  return `${days}d ${hours}h`
}

function clusterLabel(cluster?: string): string {
  if (!cluster) return '-'
  if (cluster === 'mainnet-beta') return 'mainnet'
  return cluster
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
    default:
      return common.join('\n')
  }
}

const VALIDATOR_PRESETS: Record<string, string> = {
  'mainnet-beta': buildPreset('mainnet-beta', 'validator'),
  'testnet': buildPreset('testnet', 'validator'),
  'devnet': buildPreset('devnet', 'validator'),
}

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

function parseEnvVars(text: string): Record<string, string> {
  const vars: Record<string, string> = {}
  text.split('\n').map(s => s.trim()).filter(Boolean).forEach(line => {
    const eq = line.indexOf('=')
    if (eq > 0) vars[line.slice(0, eq)] = line.slice(eq + 1)
  })
  return vars
}

function flagsToText(flags: Record<string, string>): string {
  return Object.entries(flags).map(([k, v]) => v ? `${k}=${v}` : k).join('\n')
}

function envVarsToText(vars: Record<string, string>): string {
  return Object.entries(vars).map(([k, v]) => `${k}=${v}`).join('\n')
}

function NodeDetail() {
  const { id } = useParams<{ id: string }>()
  const [node, setNode] = useState<Node | null>(null)
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [error, setError] = useState('')
  const [logFilter, setLogFilter] = useState<string>('validator')
  const [logLevel, setLogLevel] = useState<string>('all')
  const [logSearch, setLogSearch] = useState<string>('')
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
  const [provJitoRelayerUrl, setProvJitoRelayerUrl] = useState('')
  const [provJitoShredReceiverAddr, setProvJitoShredReceiverAddr] = useState('')
  const [provYellowstoneGrpc, setProvYellowstoneGrpc] = useState(false)
  const [provNoPortCheck, setProvNoPortCheck] = useState(false)
  const [provRpcPort, setProvRpcPort] = useState('8899')
  const [provDynamicPortRange, setProvDynamicPortRange] = useState('8000-8030')
  const [provSubmitting, setProvSubmitting] = useState(false)
  const [provNodeType, setProvNodeType] = useState('validator')
  const [provGossipPort, setProvGossipPort] = useState('8001')
  const [provValidatorFlags, setProvValidatorFlags] = useState(VALIDATOR_PRESETS['mainnet-beta'])
  const [provGeyserPluginConfigs, setProvGeyserPluginConfigs] = useState('')
  const [provEnvironmentVars, setProvEnvironmentVars] = useState('')
  const [provExtraArgs, setProvExtraArgs] = useState('')
  const [provRestartSec, setProvRestartSec] = useState('1')
  const [provLogRateLimitDisable, setProvLogRateLimitDisable] = useState(true)
  const [provStartLimitDisable, setProvStartLimitDisable] = useState(true)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [showProvision, setShowProvision] = useState(false)
  const [versionInfo, setVersionInfo] = useState<VersionInfo | null>(null)

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

  useEffect(() => {
    if (!id) return
    fetchNodeLogs(id, { limit: 200 })
      .then(entries => setLogs(entries.reverse()))
      .catch(() => {})
  }, [id])

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

  useEffect(() => {
    const el = logContainerRef.current
    if (el) {
      el.scrollTop = el.scrollHeight
    }
  }, [logs])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 10000)
    return () => clearInterval(interval)
  }, [refresh])

  useEffect(() => {
    fetchVersionInfo().then(setVersionInfo).catch(() => {})
  }, [])

  // Populate provision form from saved config or live node data when panel is opened
  useEffect(() => {
    if (!showProvision || !node) return

    if (node.provision_config_json) {
      try {
        const cfg = JSON.parse(node.provision_config_json) as Partial<ProvisionRequest>
        if (cfg.client) setProvClient(cfg.client)
        if (cfg.version) setProvVersion(cfg.version)
        if (cfg.cluster) setProvCluster(cfg.cluster)
        if (cfg.ledger_path) setProvLedgerPath(cfg.ledger_path)
        if (cfg.snapshot_path) setProvSnapshotPath(cfg.snapshot_path)
        if (cfg.accounts_path) setProvAccountsPath(cfg.accounts_path)
        if (cfg.identity_keypair_path) setProvIdentityPath(cfg.identity_keypair_path)
        if (cfg.vote_account_keypair_path) setProvVotePath(cfg.vote_account_keypair_path)
        if (cfg.entrypoints) setProvEntrypoints(cfg.entrypoints.join('\n'))
        if (cfg.known_validators) setProvKnownValidators(cfg.known_validators.join('\n'))
        if (cfg.download_url !== undefined) setProvDownloadUrl(cfg.download_url)
        if (cfg.sha256 !== undefined) setProvSha256(cfg.sha256)
        if (cfg.jito_mev !== undefined) setProvJitoMev(cfg.jito_mev)
        if (cfg.jito_block_engine_url !== undefined) setProvJitoBlockEngineUrl(cfg.jito_block_engine_url)
        if (cfg.jito_relayer_url !== undefined) setProvJitoRelayerUrl(cfg.jito_relayer_url)
        if (cfg.jito_shred_receiver_addr !== undefined) setProvJitoShredReceiverAddr(cfg.jito_shred_receiver_addr)
        if (cfg.yellowstone_grpc !== undefined) setProvYellowstoneGrpc(cfg.yellowstone_grpc)
        if (cfg.no_port_check !== undefined) setProvNoPortCheck(cfg.no_port_check)
        if (cfg.rpc_port) setProvRpcPort(String(cfg.rpc_port))
        if (cfg.dynamic_port_range) setProvDynamicPortRange(cfg.dynamic_port_range)
        if (cfg.node_type) setProvNodeType(cfg.node_type)
        if (cfg.gossip_port) setProvGossipPort(String(cfg.gossip_port))
        if (cfg.validator_flags) setProvValidatorFlags(flagsToText(cfg.validator_flags))
        if (cfg.geyser_plugin_configs) setProvGeyserPluginConfigs(cfg.geyser_plugin_configs.join('\n'))
        if (cfg.environment_vars) setProvEnvironmentVars(envVarsToText(cfg.environment_vars))
        if (cfg.extra_args) setProvExtraArgs(cfg.extra_args.join('\n'))
        if (cfg.restart_sec) setProvRestartSec(String(cfg.restart_sec))
        if (cfg.log_rate_limit_disable !== undefined) setProvLogRateLimitDisable(cfg.log_rate_limit_disable)
        if (cfg.start_limit_disable !== undefined) setProvStartLimitDisable(cfg.start_limit_disable)
        return
      } catch {
        // Invalid JSON — fall through to live data
      }
    }

    // No saved provision config — seed from live node data
    const s = node.live_status
    const cluster = node.cluster ?? s?.cluster
    if (cluster) {
      setProvCluster(cluster)
      setProvEntrypoints(CLUSTER_ENTRYPOINTS[cluster] || '')
      setProvKnownValidators(CLUSTER_KNOWN_VALIDATORS[cluster] || '')
      setProvValidatorFlags(buildPreset(cluster, provNodeType))
    }
    if (node.client ?? s?.client) setProvClient((node.client ?? s?.client)!)
    if (s?.version) setProvVersion(s.version)
  }, [showProvision, node?.provision_config_json, node?.node_id])

  const handleUpgradeAgent = async () => {
    if (!id || !versionInfo?.agent_update) return
    const v = versionInfo.agent_update.version
    if (!confirm(`Upgrade agent to v${v}?`)) return
    try {
      const result = await upgradeAgent(id)
      if (result.ok) {
        refresh()
      } else {
        alert(`Failed: ${result.message}`)
      }
    } catch (err) {
      alert(`Error: ${err}`)
    }
  }

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
        jito_relayer_url: provJitoRelayerUrl,
        jito_shred_receiver_addr: provJitoShredReceiverAddr,
        yellowstone_grpc: provYellowstoneGrpc,
        no_port_check: provNoPortCheck,
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
        setShowProvision(false)
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
      <div className="flex flex-col gap-4">
        <Link to="/" className="inline-flex items-center text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors mb-2">&larr; Back to Overview</Link>
        <div className="p-4 bg-red-950/30 border border-red-900/50 rounded-md text-red-400">Error loading node: {error}</div>
      </div>
    )
  }

  if (!node) {
    return (
      <div className="flex flex-col gap-4">
        <Link to="/" className="inline-flex items-center text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors mb-2">&larr; Back to Overview</Link>
        <p className="text-zinc-500">Loading...</p>
      </div>
    )
  }

  const s = node.live_status
  const hasConfig = !!(node.client || node.cluster || s?.version)

  // Logs filtered by service tab + level + free-text search
  const visibleLogs = logs.filter(
    e =>
      e.service === logFilter &&
      (logLevel === 'all' || e.level === logLevel) &&
      (logSearch.trim() === '' || e.message.toLowerCase().includes(logSearch.toLowerCase()))
  )

  return (
    <div className="flex flex-col gap-8 max-w-6xl mx-auto">
      <Link to="/" className="inline-flex items-center text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors w-max">&larr; Back to Overview</Link>

      {/* Header */}
      <div className="flex flex-wrap items-center gap-4 bg-[#15131f] border border-white/10 rounded-xl p-6 shadow-sm">
        <h1 className="text-2xl font-semibold text-zinc-100 m-0 leading-none">{node.node_id}</h1>
        <div className="flex items-center gap-2">
          <span className={`inline-flex items-center px-2 py-0.5 text-[11px] font-medium uppercase tracking-wider rounded border ${STATE_BADGE_CLASSES[node.lifecycle_state] || 'bg-zinc-500/10 text-zinc-400 border-zinc-500/20'}`}>
            {node.lifecycle_state}
          </span>
          <span className="flex items-center gap-1.5 px-2 py-0.5 text-[11px] font-medium uppercase tracking-wider rounded border bg-zinc-800/50 border-zinc-700/50 text-zinc-400">
            <div className={`w-1.5 h-1.5 rounded-full ${node.live_status ? 'bg-green-500 shadow-[0_0_8px_rgba(34,197,94,0.4)]' : 'bg-red-500'}`}></div>
            {node.live_status ? 'Connected' : 'Disconnected'}
          </span>
        </div>
        
        {node.hostname && node.hostname !== node.node_id && (
          <span className="text-sm font-mono text-zinc-500">{node.hostname}</span>
        )}
        <span className="text-sm text-zinc-500">Last seen: {formatLastSeen(node.last_seen_at)}</span>
        
        <div className="ml-auto">
          <a
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium text-purple-400 bg-purple-500/10 border border-purple-500/20 rounded-md hover:bg-purple-500/20 transition-colors"
            href={`/grafana/d/pillar-node-detail/pillar-node-detail?orgId=1&from=now-1h&to=now&timezone=browser&var-datasource=pillar-prometheus&var-node_id=${encodeURIComponent(node.node_id)}&refresh=30s`}
            target="_blank"
            rel="noopener noreferrer"
          >
            Metrics ↗
          </a>
        </div>
      </div>

      {/* Node Info Cards - versions, client, cluster */}
      <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
        <div className="flex flex-col bg-purple-900/10 border border-purple-500/20 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-purple-400/80 uppercase tracking-wider mb-2">Validator Version</div>
          <div className="text-lg font-mono font-medium text-purple-100">{s?.version || '-'}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Agent Version</div>
          <div className="text-lg font-mono text-zinc-300">{node.agent_version || '-'}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Client</div>
          <div className="text-lg text-zinc-300 capitalize">{node.client ?? s?.client ?? '-'}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Cluster</div>
          <div className="text-lg text-zinc-300">
            {(node.cluster || s?.cluster) ? (
              <span className="inline-flex items-center px-2 py-0.5 text-xs font-medium bg-zinc-800 text-zinc-300 rounded border border-zinc-700">
                {clusterLabel(node.cluster ?? s?.cluster)}
              </span>
            ) : '-'}
          </div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Role</div>
          <div className="text-lg text-zinc-300 capitalize">{node.role ?? s?.role ?? '-'}</div>
        </div>
      </div>

      {/* Metrics */}
      <div className="grid grid-cols-2 md:grid-cols-6 gap-4">
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Slots Behind</div>
          <div className="text-xl font-mono text-zinc-200">{s?.slots_behind ?? '-'}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">CPU</div>
          <div className="text-xl font-mono text-zinc-200">{s ? `${s.cpu_usage_percent.toFixed(1)}%` : '-'}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Memory</div>
          <div className="text-base font-mono text-zinc-300">
            {s ? `${formatBytes(s.memory_used_bytes)} / ${formatBytes(s.memory_total_bytes)}` : '-'}
          </div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Disk</div>
          <div className="text-base font-mono text-zinc-300">
            {s ? `${formatBytes(s.disk_used_bytes)} / ${formatBytes(s.disk_total_bytes)}` : '-'}
          </div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Restarts</div>
          <div className="text-xl font-mono text-zinc-200">{s?.restart_count ?? '-'}</div>
        </div>
        <div className="flex flex-col bg-[#15131f] border border-white/10 rounded-xl p-5 shadow-sm">
          <div className="text-xs font-medium text-zinc-400 uppercase tracking-wider mb-2">Running Since</div>
          <div className="text-base font-mono text-zinc-300">{s?.state_duration_secs != null ? formatDuration(s.state_duration_secs) : '-'}</div>
        </div>
      </div>

      {/* Current Config (read-only) */}
      {hasConfig && (
        <div className="bg-[#15131f] border border-white/10 rounded-xl p-6 shadow-sm">
          <h2 className="text-lg font-semibold text-zinc-100 mb-4">Current Configuration</h2>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-6 p-4 bg-black/20 rounded-lg border border-white/5">
            {node.client && (
              <div className="flex flex-col gap-1">
                <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Client</span>
                <span className="text-sm font-mono text-zinc-300">{node.client}</span>
              </div>
            )}
            {(node.cluster || s?.cluster) && (
              <div className="flex flex-col gap-1">
                <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Cluster</span>
                <span className="text-sm font-mono text-zinc-300">{node.cluster ?? s?.cluster}</span>
              </div>
            )}
            {s?.version && (
              <div className="flex flex-col gap-1">
                <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Validator Version</span>
                <span className="text-sm font-mono text-zinc-300">{s.version}</span>
              </div>
            )}
            {node.agent_version && (
              <div className="flex flex-col gap-1">
                <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Agent Version</span>
                <span className="text-sm font-mono text-zinc-300">{node.agent_version}</span>
              </div>
            )}
            {(node.role || s?.role) && (
              <div className="flex flex-col gap-1">
                <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">Role</span>
                <span className="text-sm font-mono text-zinc-300">{node.role ?? s?.role}</span>
              </div>
            )}
            {node.ip_address && (
              <div className="flex flex-col gap-1">
                <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">IP Address</span>
                <span className="text-sm font-mono text-zinc-300">{node.ip_address}</span>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Provision - opens in modal */}
      <div className="bg-[#15131f] border border-purple-500/20 rounded-xl p-6 shadow-[0_0_20px_rgba(153,69,255,0.05)]">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-lg font-semibold text-zinc-100 mb-1">{hasConfig ? 'Update Validator' : 'Setup Validator'}</h2>
            <p className="m-0 text-sm text-zinc-400">
              {hasConfig
                ? 'Change the client, version, cluster, ports, or flags and re-deploy this validator.'
                : 'Install and configure a validator on this host — pick a client, cluster, version, and ports.'}
            </p>
          </div>
          <button 
            className="px-4 py-2 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all whitespace-nowrap" 
            onClick={() => setShowProvision(true)}
          >
            {hasConfig ? 'Update Validator' : 'Configure Validator'}
          </button>
        </div>
      </div>

      {showProvision && (
        <div className="fixed inset-0 z-50 flex justify-center py-10 px-4 bg-black/60 backdrop-blur-sm overflow-y-auto" onClick={() => setShowProvision(false)}>
          <div className="w-full max-w-4xl p-6 bg-[#15131f] border border-white/10 rounded-xl shadow-2xl flex flex-col gap-5 m-auto h-max" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-white/5 pb-4">
              <h2 className="text-xl font-semibold text-zinc-100 m-0">{hasConfig ? 'Update Validator' : 'Setup Validator'}</h2>
              <button className="px-3 py-1.5 text-xs font-medium text-zinc-400 hover:text-zinc-200 transition-colors bg-white/5 hover:bg-white/10 rounded-md border border-white/10" onClick={() => setShowProvision(false)}>Close</button>
            </div>
            
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Client</label>
                <select className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all appearance-none" value={provClient} onChange={e => setProvClient(e.target.value)}>
                  <option value="agave">Agave</option>
                  <option value="jito">Jito</option>
                  <option value="firedancer">Firedancer</option>
                  <option value="frankendancer">Frankendancer</option>
                  <option value="surfpool">Surfpool (test validator)</option>
                </select>
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Cluster</label>
                <select className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all appearance-none" value={provCluster} onChange={e => handleClusterChange(e.target.value)}>
                  <option value="mainnet-beta">mainnet-beta</option>
                  <option value="testnet">testnet</option>
                  <option value="devnet">devnet</option>
                </select>
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Node Type</label>
                <select className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all appearance-none" value={provNodeType} onChange={e => handleNodeTypeChange(e.target.value)}>
                  <option value="validator">Validator</option>
                  <option value="rpc">RPC Node</option>
                  <option value="archival">Archival RPC</option>
                </select>
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Version</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provVersion} onChange={e => setProvVersion(e.target.value)} placeholder="e.g. 2.1.6" />
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 mt-2">
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Ledger Path</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provLedgerPath} onChange={e => setProvLedgerPath(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Snapshot Path</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provSnapshotPath} onChange={e => setProvSnapshotPath(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Accounts Path</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provAccountsPath} onChange={e => setProvAccountsPath(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Identity Keypair</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provIdentityPath} onChange={e => setProvIdentityPath(e.target.value)} />
              </div>
              {!noVotingActive && (
                <div className="flex flex-col gap-1.5">
                  <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Vote Keypair</label>
                  <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provVotePath} onChange={e => setProvVotePath(e.target.value)} placeholder="/home/sol/vote-account-keypair.json" />
                </div>
              )}
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">RPC Port</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provRpcPort} onChange={e => setProvRpcPort(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Gossip Port</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provGossipPort} onChange={e => setProvGossipPort(e.target.value)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Dynamic Port Range</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provDynamicPortRange} onChange={e => setProvDynamicPortRange(e.target.value)} />
              </div>
            </div>

            <div className="flex flex-col gap-1.5 mt-2">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Entrypoints</label>
              <textarea className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600 resize-y" rows={4} value={provEntrypoints} onChange={e => setProvEntrypoints(e.target.value)} placeholder="One per line" />
            </div>
            <div className="flex flex-col gap-1.5 mt-2">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Known Validators</label>
              <textarea className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600 resize-y" rows={3} value={provKnownValidators} onChange={e => setProvKnownValidators(e.target.value)} placeholder="One pubkey per line" />
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-2">
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Download URL</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provDownloadUrl} onChange={e => setProvDownloadUrl(e.target.value)} placeholder="https://..." />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">SHA256</label>
                <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provSha256} onChange={e => setProvSha256(e.target.value)} placeholder="Expected hash" />
              </div>
            </div>

            <div className="flex flex-wrap gap-6 mt-2 p-4 bg-white/[0.02] border border-white/5 rounded-lg">
              <div className="flex flex-col gap-2 w-full md:w-auto">
                <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                  <input className="rounded border-white/20 bg-black/40 text-purple-500 focus:ring-purple-500/50" type="checkbox" checked={provJitoMev} onChange={e => setProvJitoMev(e.target.checked)} />
                  Jito MEV
                </label>
                {provJitoMev && (
                  <div className="flex flex-col gap-2 mt-2 w-full max-w-sm">
                    <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provJitoBlockEngineUrl} onChange={e => setProvJitoBlockEngineUrl(e.target.value)} placeholder="Block Engine URL" />
                    <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provJitoRelayerUrl} onChange={e => setProvJitoRelayerUrl(e.target.value)} placeholder="Relayer URL (optional)" />
                    <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provJitoShredReceiverAddr} onChange={e => setProvJitoShredReceiverAddr(e.target.value)} placeholder="Shred Receiver host:port (optional)" />
                  </div>
                )}
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                  <input className="rounded border-white/20 bg-black/40 text-purple-500 focus:ring-purple-500/50" type="checkbox" checked={provYellowstoneGrpc} onChange={e => setProvYellowstoneGrpc(e.target.checked)} />
                  Yellowstone gRPC
                </label>
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                  <input className="rounded border-white/20 bg-black/40 text-purple-500 focus:ring-purple-500/50" type="checkbox" checked={provNoPortCheck} onChange={e => setProvNoPortCheck(e.target.checked)} />
                  Skip port check (NAT/firewall)
                </label>
              </div>
            </div>

            {/* Advanced Settings */}
            <div className="mt-2 border-t border-white/5 pt-4">
              <button
                className="px-3 py-1.5 text-xs font-medium text-zinc-400 bg-white/5 hover:bg-white/10 border border-white/10 rounded-md transition-colors"
                onClick={() => setShowAdvanced(!showAdvanced)}
              >
                {showAdvanced ? 'Hide' : 'Show'} Advanced
              </button>

              {showAdvanced && (
                <div className="mt-4 flex flex-col gap-4">
                  <div className="flex flex-col gap-1.5">
                    <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Validator Flags <span className="lowercase text-zinc-500 font-normal">(one per line)</span></label>
                    <textarea
                      className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600 resize-y"
                      rows={6}
                      value={provValidatorFlags}
                      onChange={e => setProvValidatorFlags(e.target.value)}
                      placeholder="no-port-check&#10;limit-ledger-size&#10;rpc-bind-address=0.0.0.0"
                    />
                  </div>

                  <div className="flex flex-col gap-1.5">
                    <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Geyser Plugin Configs</label>
                    <textarea className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600 resize-y" rows={2} value={provGeyserPluginConfigs} onChange={e => setProvGeyserPluginConfigs(e.target.value)} placeholder="/etc/pillar/custom-geyser.json" />
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                    <div className="flex flex-col gap-1.5">
                      <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">RestartSec</label>
                      <input className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600" type="text" value={provRestartSec} onChange={e => setProvRestartSec(e.target.value)} />
                    </div>
                    <div className="flex flex-col gap-1.5 justify-center">
                      <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                        <input className="rounded border-white/20 bg-black/40 text-purple-500 focus:ring-purple-500/50" type="checkbox" checked={provLogRateLimitDisable} onChange={e => setProvLogRateLimitDisable(e.target.checked)} />
                        Disable Log Rate Limit
                      </label>
                    </div>
                    <div className="flex flex-col gap-1.5 justify-center">
                      <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                        <input className="rounded border-white/20 bg-black/40 text-purple-500 focus:ring-purple-500/50" type="checkbox" checked={provStartLimitDisable} onChange={e => setProvStartLimitDisable(e.target.checked)} />
                        Disable Start Limit
                      </label>
                    </div>
                  </div>

                  <div className="flex flex-col gap-1.5">
                    <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Environment Variables <span className="lowercase text-zinc-500 font-normal">(KEY=VALUE)</span></label>
                    <textarea className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600 resize-y" rows={2} value={provEnvironmentVars} onChange={e => setProvEnvironmentVars(e.target.value)} placeholder="SOLANA_METRICS_CONFIG=host=https://metrics.solana.com:8086,db=mainnet-beta" />
                  </div>

                  <div className="flex flex-col gap-1.5">
                    <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Extra CLI Arguments</label>
                    <textarea className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600 resize-y" rows={2} value={provExtraArgs} onChange={e => setProvExtraArgs(e.target.value)} placeholder="--custom-flag value" />
                  </div>
                </div>
              )}
            </div>

            <div className="flex items-center justify-end gap-3 mt-4 border-t border-white/5 pt-6">
              <button className="px-4 py-2 text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors" onClick={() => setShowProvision(false)} disabled={provSubmitting}>Cancel</button>
              <button className="px-5 py-2 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all disabled:opacity-50" onClick={handleProvision} disabled={provSubmitting}>
                {provSubmitting ? 'Sending...' : (hasConfig ? 'Update Validator' : 'Install Validator')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Logs */}
      <div className="bg-[#15131f] border border-white/10 rounded-xl overflow-hidden flex flex-col h-[500px] shadow-sm">
        <div className="flex flex-wrap items-center justify-between gap-4 p-4 bg-white/[0.02] border-b border-white/10">
          <h2 className="text-lg font-semibold text-zinc-100 m-0">Logs</h2>
          <div className="flex items-center gap-3">
            <input
              type="text"
              value={logSearch}
              onChange={e => setLogSearch(e.target.value)}
              placeholder="Filter messages..."
              className="w-48 px-3 py-1.5 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-600"
            />
            <select
              value={logLevel}
              onChange={e => setLogLevel(e.target.value)}
              className="px-3 py-1.5 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all appearance-none pr-8 cursor-pointer"
            >
              <option value="all">All levels</option>
              <option value="error">Error</option>
              <option value="warn">Warn</option>
              <option value="info">Info</option>
              <option value="debug">Debug</option>
            </select>
            <span className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded-full border ${sseConnected ? 'bg-green-500/10 text-green-400 border-green-500/20' : 'bg-zinc-500/10 text-zinc-400 border-zinc-500/20'}`}>
              <div className={`w-1.5 h-1.5 rounded-full ${sseConnected ? 'bg-green-500 shadow-[0_0_8px_rgba(34,197,94,0.4)]' : 'bg-zinc-500'}`}></div>
              {sseConnected ? 'Live' : 'Disconnected'}
            </span>
          </div>
        </div>
        <div className="flex bg-black/20 border-b border-white/10 px-4">
          {['controller', 'validator', 'agent'].map(tab => (
            <button
              key={tab}
              className={`px-4 py-2.5 text-sm font-medium border-b-2 transition-colors ${logFilter === tab ? 'text-purple-400 border-purple-500' : 'text-zinc-400 hover:text-zinc-200 border-transparent'}`}
              onClick={() => setLogFilter(tab)}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
        <div className="flex-1 overflow-y-auto p-4 bg-black/40 font-mono text-[13px] leading-relaxed" ref={logContainerRef}>
          {visibleLogs.length === 0 && (
            <div className="text-zinc-500 text-center py-8">
              {logs.filter(e => e.service === logFilter).length === 0 ? 'No logs available' : 'No logs match the filter'}
            </div>
          )}
          {visibleLogs.map((entry) => (
            <div key={entry.id} className="flex gap-4 py-1 hover:bg-white/[0.02] border-b border-white/5 last:border-0 transition-colors">
              <span className="text-zinc-500 shrink-0 select-none">{formatTimestamp(entry.timestamp_ms)}</span>
              <span className="text-purple-400/80 shrink-0 w-20 truncate">{entry.service}</span>
              <span className={`shrink-0 w-12 font-semibold ${entry.level === 'error' ? 'text-red-400' : entry.level === 'warn' ? 'text-yellow-400' : entry.level === 'info' ? 'text-green-400' : 'text-zinc-400'}`}>{entry.level.toUpperCase().padEnd(5)}</span>
              <span className="text-zinc-300 break-words flex-1 whitespace-pre-wrap">{entry.message}</span>
            </div>
          ))}
        </div>
      </div>

      {/* Actions — at the bottom (destructive/lifecycle controls) */}
      <div className="mt-4 pt-6 border-t border-white/5">
        <h3 className="text-sm font-medium text-zinc-400 uppercase tracking-wider mb-4">Actions</h3>
        <div className="flex flex-wrap gap-3">
          <button className="px-4 py-2 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all" onClick={handleRestart}>Restart</button>
          <button className="px-4 py-2 text-sm font-medium text-zinc-300 bg-white/5 hover:bg-white/10 rounded-md border border-white/10 shadow-sm transition-all" onClick={handleRecover}>Recover</button>
          <button className="px-4 py-2 text-sm font-medium text-zinc-300 bg-white/5 hover:bg-white/10 rounded-md border border-white/10 shadow-sm transition-all" onClick={handleStop}>Stop</button>
          {versionInfo?.agent_update && node.agent_version && node.agent_version !== versionInfo.agent_update.version && (
            <button className="px-4 py-2 text-sm font-medium text-white bg-green-600 hover:bg-green-500 rounded-md border border-green-500/50 shadow-sm transition-all" onClick={handleUpgradeAgent}>
              Upgrade Agent to v{versionInfo.agent_update.version}
            </button>
          )}
          {(node.lifecycle_state === 'provisioning' || node.lifecycle_state === 'starting_up') && (
            <button className="px-4 py-2 text-sm font-medium text-red-400 bg-red-950/30 border border-red-900/50 rounded-md hover:bg-red-900/30 transition-all ml-auto" onClick={handleCancel}>Cancel</button>
          )}
          <button className={`px-4 py-2 text-sm font-medium text-red-400 bg-red-950/30 border border-red-900/50 rounded-md hover:bg-red-900/30 transition-all ${!((node.lifecycle_state === 'provisioning' || node.lifecycle_state === 'starting_up')) ? 'ml-auto' : ''}`} onClick={handleDelete}>Delete</button>
        </div>
      </div>
    </div>
  )
}

export default NodeDetail
