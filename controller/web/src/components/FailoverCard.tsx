import { useState, useEffect, useCallback } from 'react'
import { Link } from 'react-router-dom'
import {
  fetchFailoverPairs,
  fetchNodes,
  createFailoverPair,
  deleteFailoverPair,
  setFailoverAuto,
  triggerFailover,
  rerunFailoverPrepare,
} from '../api'
import type { FailoverPair, Node } from '../api'

const FAILOVER_CLIENTS = ['agave', 'jito']

const PREPARE_STATE_LABELS: Record<string, { label: string; cls: string }> = {
  preparing: { label: 'Preparing…', cls: 'bg-yellow-500/10 text-yellow-500 border-yellow-500/20' },
  primary_ready: { label: 'Waiting for backup…', cls: 'bg-yellow-500/10 text-yellow-500 border-yellow-500/20' },
  backup_ready: { label: 'Waiting for primary…', cls: 'bg-yellow-500/10 text-yellow-500 border-yellow-500/20' },
  ready: { label: 'Ready', cls: 'bg-green-500/10 text-green-400 border-green-500/20' },
  prepare_failed: { label: 'Prepare failed', cls: 'bg-red-500/10 text-red-400 border-red-500/20' },
}

const OP_STATE_LABELS: Record<string, string> = {
  pending_demote: 'Demoting primary (waiting for a safe restart window)…',
  pending_promote: 'Promoting backup…',
  complete: 'Completed',
  failed: 'Failed',
}

function opIsActive(state: string): boolean {
  return state === 'pending_demote' || state === 'pending_promote'
}

interface Props {
  nodeId: string
}

function FailoverCard({ nodeId }: Props) {
  const [pair, setPair] = useState<FailoverPair | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [showForm, setShowForm] = useState(false)
  const [candidates, setCandidates] = useState<Node[]>([])
  const [backupId, setBackupId] = useState('')
  const [stakedPath, setStakedPath] = useState('')
  const [unstakedPath, setUnstakedPath] = useState('/home/sol/unstaked-identity.json')
  const [symlinkPath, setSymlinkPath] = useState('/home/sol/pillar-identity.json')
  const [submitting, setSubmitting] = useState(false)
  const [busy, setBusy] = useState(false)
  const [showCrashConfirm, setShowCrashConfirm] = useState(false)
  const [crashConfirmText, setCrashConfirmText] = useState('')

  const refresh = useCallback(async () => {
    try {
      const pairs = await fetchFailoverPairs()
      setPair(pairs.find(p => p.primary_node_id === nodeId || p.backup_node_id === nodeId) ?? null)
      setLoaded(true)
    } catch {
      // keep last known state
    }
  }, [nodeId])

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 10000)
    return () => clearInterval(interval)
  }, [refresh])

  // Load eligible backup nodes when the pairing form opens
  useEffect(() => {
    if (!showForm) return
    Promise.all([fetchNodes(), fetchFailoverPairs()])
      .then(([nodes, pairs]) => {
        const pairedIds = new Set(pairs.flatMap(p => [p.primary_node_id, p.backup_node_id]))
        const self = nodes.find(n => n.node_id === nodeId)
        const selfCluster = self?.cluster
        setCandidates(
          nodes.filter(
            n =>
              n.node_id !== nodeId &&
              !pairedIds.has(n.node_id) &&
              !!n.provision_config_json &&
              FAILOVER_CLIENTS.includes(n.client ?? '') &&
              n.cluster === selfCluster,
          ),
        )
        // Prefill staked path from this node's provision config
        if (self?.provision_config_json) {
          try {
            const cfg = JSON.parse(self.provision_config_json)
            if (cfg.identity_keypair_path) setStakedPath(cfg.identity_keypair_path)
          } catch {
            // leave default
          }
        }
      })
      .catch(() => setCandidates([]))
  }, [showForm, nodeId])

  const handleCreate = async () => {
    if (!backupId) {
      alert('Select a backup node.')
      return
    }
    if (
      !confirm(
        `Pair ${nodeId} (primary) with ${backupId} (backup)?\n\n` +
          `Both validators will be reconfigured to start against an identity symlink, ` +
          `and the backup will be restarted with a generated unstaked identity.`,
      )
    )
      return
    setSubmitting(true)
    try {
      await createFailoverPair({
        primary_node_id: nodeId,
        backup_node_id: backupId,
        staked_identity_path: stakedPath || undefined,
        unstaked_identity_path: unstakedPath || undefined,
        symlink_path: symlinkPath || undefined,
      })
      setShowForm(false)
      refresh()
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setSubmitting(false)
    }
  }

  const handleDelete = async () => {
    if (!pair) return
    if (!confirm('Remove this failover pairing? The nodes keep their current identities; nothing is changed on the machines.')) return
    setBusy(true)
    try {
      await deleteFailoverPair(pair.pair_id)
      refresh()
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setBusy(false)
    }
  }

  const handleToggleAuto = async () => {
    if (!pair) return
    setBusy(true)
    try {
      await setFailoverAuto(pair.pair_id, !pair.auto_failover)
      refresh()
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setBusy(false)
    }
  }

  const handleGraceful = async () => {
    if (!pair) return
    if (
      !confirm(
        `Fail over now?\n\nThe primary (${pair.primary_node_id}) will wait for a safe restart window, ` +
          `hand its voting identity and tower file to ${pair.backup_node_id}, and keep running unstaked.`,
      )
    )
      return
    setBusy(true)
    try {
      await triggerFailover(pair.pair_id, 'graceful')
      refresh()
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setBusy(false)
    }
  }

  const handleCrash = async (force: boolean) => {
    if (!pair) return
    setBusy(true)
    try {
      await triggerFailover(pair.pair_id, 'crash', force)
      setShowCrashConfirm(false)
      setCrashConfirmText('')
      refresh()
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setBusy(false)
    }
  }

  const handleRerunPrepare = async () => {
    if (!pair) return
    setBusy(true)
    try {
      await rerunFailoverPrepare(pair.pair_id)
      refresh()
    } catch (err) {
      alert(`Error: ${err}`)
    } finally {
      setBusy(false)
    }
  }

  if (!loaded) return null

  const isPrimary = pair?.primary_node_id === nodeId
  const peerId = pair ? (isPrimary ? pair.backup_node_id : pair.primary_node_id) : ''
  const prepareChip = pair ? PREPARE_STATE_LABELS[pair.prepare_state] : undefined
  const op = pair?.last_op
  const activeOp = op && opIsActive(op.state)
  const pubkeyPrefix = pair?.staked_pubkey?.slice(0, 8) ?? ''

  return (
    <div className="bg-[#15131f] border border-white/10 rounded-xl p-6 shadow-sm">
      <div className="flex items-center justify-between gap-4 mb-4">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100 m-0">Failover</h2>
          <p className="text-sm text-zinc-400 mt-1 m-0">
            Hot-spare identity swap — only the voting identity moves between machines.
          </p>
        </div>
        {!pair && !showForm && (
          <button
            className="px-4 py-2 text-sm font-medium text-purple-300 bg-purple-500/10 hover:bg-purple-500/20 rounded-md border border-purple-500/20 shadow-sm transition-all whitespace-nowrap"
            onClick={() => setShowForm(true)}
          >
            Enable Hot-Spare Failover
          </button>
        )}
      </div>

      {/* Unpaired: pairing form */}
      {!pair && showForm && (
        <div className="flex flex-col gap-4">
          <div className="p-3 bg-yellow-500/5 border border-yellow-500/20 rounded-md text-sm text-yellow-200/80">
            Before enabling: copy the staked identity keypair to the backup machine at the{' '}
            <span className="font-mono">same path</span>. Pillar never transfers private keys.
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Backup Node</label>
              <select
                className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all appearance-none"
                value={backupId}
                onChange={e => setBackupId(e.target.value)}
              >
                <option value="">Select a node…</option>
                {candidates.map(n => (
                  <option key={n.node_id} value={n.node_id}>
                    {n.node_id} ({n.client}, {n.cluster})
                  </option>
                ))}
              </select>
              {candidates.length === 0 && (
                <span className="text-xs text-zinc-500">
                  No eligible nodes (must be provisioned agave/jito, same cluster, not already paired).
                </span>
              )}
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Staked Identity Path</label>
              <input
                className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all"
                type="text"
                value={stakedPath}
                onChange={e => setStakedPath(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Unstaked Identity Path</label>
              <input
                className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all"
                type="text"
                value={unstakedPath}
                onChange={e => setUnstakedPath(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Identity Symlink Path</label>
              <input
                className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-purple-500/50 transition-all"
                type="text"
                value={symlinkPath}
                onChange={e => setSymlinkPath(e.target.value)}
              />
            </div>
          </div>
          <div className="flex items-center gap-3">
            <button
              className="px-4 py-2 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md shadow-sm transition-all disabled:opacity-50"
              onClick={handleCreate}
              disabled={submitting || !backupId}
            >
              {submitting ? 'Pairing…' : 'Create Pair & Prepare'}
            </button>
            <button
              className="px-3 py-2 text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors"
              onClick={() => setShowForm(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {!pair && !showForm && (
        <p className="text-sm text-zinc-500 m-0">
          Not paired. Pair this validator with a hot-spare so the voting identity can move if it goes down.
        </p>
      )}

      {/* Paired */}
      {pair && (
        <div className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center gap-3">
            <span
              className={`inline-flex items-center px-2 py-0.5 text-[11px] font-medium uppercase tracking-wider rounded border ${
                isPrimary
                  ? 'bg-amber-500/10 text-amber-400 border-amber-500/20'
                  : 'bg-blue-500/10 text-blue-400 border-blue-500/20'
              }`}
            >
              {isPrimary ? 'Primary (staked voter)' : 'Backup (hot spare)'}
            </span>
            {prepareChip && (
              <span className={`inline-flex items-center px-2 py-0.5 text-[11px] font-medium uppercase tracking-wider rounded border ${prepareChip.cls}`}>
                {prepareChip.label}
              </span>
            )}
            <span className="text-sm text-zinc-400">
              Paired with{' '}
              <Link to={`/nodes/${encodeURIComponent(peerId)}`} className="text-purple-400 hover:text-purple-300 font-mono">
                {peerId}
              </Link>
            </span>
            {pair.staked_pubkey && (
              <span className="text-xs text-zinc-500 font-mono" title={pair.staked_pubkey}>
                identity {pair.staked_pubkey.slice(0, 8)}…
              </span>
            )}
          </div>

          {pair.prepare_error && pair.prepare_state === 'prepare_failed' && (
            <div className="p-3 bg-red-950/30 border border-red-900/50 rounded-md text-sm text-red-400">
              {pair.prepare_error}
            </div>
          )}

          {pair.pending_cold_demote_node_id && (
            <div className="p-3 bg-yellow-500/5 border border-yellow-500/20 rounded-md text-sm text-yellow-200/80">
              Waiting for crashed ex-primary <span className="font-mono">{pair.pending_cold_demote_node_id}</span> to
              reconnect so it can be demoted to the unstaked identity.
            </div>
          )}

          {op && (activeOp || op.state === 'failed') && (
            <div
              className={`p-3 rounded-md text-sm border ${
                op.state === 'failed'
                  ? 'bg-red-950/30 border-red-900/50 text-red-400'
                  : 'bg-blue-500/5 border-blue-500/20 text-blue-300'
              }`}
            >
              <span className="font-medium">
                {op.kind === 'crash' ? 'Crash failover' : 'Failover'}: {OP_STATE_LABELS[op.state] ?? op.state}
              </span>
              {op.error && <div className="mt-1 text-xs whitespace-pre-wrap">{op.error}</div>}
            </div>
          )}

          <div className="flex flex-wrap items-center gap-3">
            <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer select-none">
              <input type="checkbox" checked={pair.auto_failover} onChange={handleToggleAuto} disabled={busy} />
              Auto-failover when the primary crashes
            </label>

            <div className="w-px h-6 bg-white/10 mx-1"></div>

            <button
              className="px-3 py-1.5 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md shadow-sm transition-all disabled:opacity-50"
              onClick={handleGraceful}
              disabled={busy || pair.prepare_state !== 'ready' || !!activeOp}
              title="Graceful identity swap with tower handoff"
            >
              Failover Now
            </button>
            <button
              className="px-3 py-1.5 text-sm font-medium text-red-400 bg-red-950/30 border border-red-900/50 rounded-md hover:bg-red-900/30 transition-all disabled:opacity-50"
              onClick={() => setShowCrashConfirm(true)}
              disabled={busy || pair.prepare_state !== 'ready' || !!activeOp}
              title="Promote the backup without a tower file (primary dead)"
            >
              Force Promote (no tower)
            </button>
            {pair.prepare_state === 'prepare_failed' && (
              <button
                className="px-3 py-1.5 text-sm font-medium text-zinc-300 bg-white/5 hover:bg-white/10 rounded-md border border-white/10 shadow-sm transition-all disabled:opacity-50"
                onClick={handleRerunPrepare}
                disabled={busy}
              >
                Retry Prepare
              </button>
            )}
            <button
              className="px-3 py-1.5 text-sm font-medium text-red-400 bg-red-950/30 border border-red-900/50 rounded-md hover:bg-red-900/30 transition-all disabled:opacity-50"
              onClick={handleDelete}
              disabled={busy || !!activeOp}
            >
              Remove Pairing
            </button>
          </div>

          <p className="text-xs text-zinc-500 m-0">
            Validate a graceful swap on testnet before relying on this for mainnet. Crash promotion skips the tower
            file and must only be used when the primary is truly down.
          </p>
        </div>
      )}

      {/* Crash/force confirmation modal */}
      {showCrashConfirm && pair && (
        <div
          className="fixed inset-0 z-50 flex justify-center items-center px-4 bg-black/60 backdrop-blur-sm"
          onClick={() => setShowCrashConfirm(false)}
        >
          <div
            className="w-full max-w-lg p-6 bg-[#15131f] border border-red-900/50 rounded-xl shadow-2xl flex flex-col gap-4"
            onClick={e => e.stopPropagation()}
          >
            <h3 className="text-lg font-semibold text-red-400 m-0">Promote backup without tower file</h3>
            <p className="text-sm text-zinc-300 m-0">
              This promotes <span className="font-mono">{pair.backup_node_id}</span> to the staked identity{' '}
              <span className="font-mono">WITHOUT</span> the primary's voting history (tower file). If the primary is
              still voting, this can cause a slashable double vote. Only continue if{' '}
              <span className="font-mono">{pair.primary_node_id}</span> is truly down.
            </p>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">
                Type the identity prefix <span className="font-mono text-zinc-300">{pubkeyPrefix}</span> to confirm
              </label>
              <input
                className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm font-mono focus:outline-none focus:border-red-500/50 transition-all"
                type="text"
                value={crashConfirmText}
                onChange={e => setCrashConfirmText(e.target.value)}
                placeholder={pubkeyPrefix}
              />
            </div>
            <div className="flex items-center gap-3">
              <button
                className="px-4 py-2 text-sm font-medium text-white bg-red-600 hover:bg-red-500 rounded-md shadow-sm transition-all disabled:opacity-50"
                onClick={() => handleCrash(true)}
                disabled={busy || crashConfirmText !== pubkeyPrefix || pubkeyPrefix === ''}
              >
                Promote Without Tower
              </button>
              <button
                className="px-3 py-2 text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors"
                onClick={() => {
                  setShowCrashConfirm(false)
                  setCrashConfirmText('')
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default FailoverCard
