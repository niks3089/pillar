import { useState, useEffect, useCallback } from 'react'
import {
  fetchAlertRules,
  updateAlertRule,
  createAlertRule,
  deleteAlertRule,
  fetchAlertHistory,
  fetchActiveAlerts,
  fetchNotificationChannels,
  createNotificationChannel,
  updateNotificationChannel,
  deleteNotificationChannel,
  testNotificationChannel,
} from '../api'
import type { AlertRule, AlertHistoryEntry, NotificationChannel } from '../api'

type Tab = 'rules' | 'channels' | 'history'

const SEVERITY_COLORS: Record<string, string> = {
  critical: 'var(--red)',
  warning: 'var(--yellow)',
  info: 'var(--blue)',
}

const OPERATOR_LABELS: Record<string, string> = {
  eq: '==',
  neq: '!=',
  gt: '>',
  gte: '>=',
  lt: '<',
  lte: '<=',
}

function formatTime(epoch: number): string {
  if (!epoch) return '-'
  const d = new Date(epoch * 1000)
  return d.toLocaleString()
}

function Alerts() {
  const [tab, setTab] = useState<Tab>('rules')
  const [rules, setRules] = useState<AlertRule[]>([])
  const [history, setHistory] = useState<AlertHistoryEntry[]>([])
  const [activeAlerts, setActiveAlerts] = useState<AlertHistoryEntry[]>([])
  const [channels, setChannels] = useState<NotificationChannel[]>([])
  const [editingThreshold, setEditingThreshold] = useState<string | null>(null)
  const [thresholdValue, setThresholdValue] = useState('')
  const [showCreateRule, setShowCreateRule] = useState(false)
  const [showCreateChannel, setShowCreateChannel] = useState(false)
  const [testResult, setTestResult] = useState<Record<string, string>>({})

  const loadRules = useCallback(async () => {
    try {
      const r = await fetchAlertRules()
      setRules(r)
    } catch { /* ignore */ }
  }, [])

  const loadHistory = useCallback(async () => {
    try {
      const [h, a] = await Promise.all([fetchAlertHistory({ limit: 200 }), fetchActiveAlerts()])
      setHistory(h)
      setActiveAlerts(a)
    } catch { /* ignore */ }
  }, [])

  const loadChannels = useCallback(async () => {
    try {
      const c = await fetchNotificationChannels()
      setChannels(c)
    } catch { /* ignore */ }
  }, [])

  useEffect(() => {
    loadRules()
    loadChannels()
    loadHistory()
  }, [loadRules, loadChannels, loadHistory])

  useEffect(() => {
    if (tab === 'history') {
      const interval = setInterval(loadHistory, 10000)
      return () => clearInterval(interval)
    }
  }, [tab, loadHistory])

  const toggleRule = async (rule: AlertRule) => {
    await updateAlertRule(rule.id, { enabled: !rule.enabled })
    loadRules()
  }

  const saveThreshold = async (ruleId: string) => {
    await updateAlertRule(ruleId, { threshold: thresholdValue })
    setEditingThreshold(null)
    loadRules()
  }

  const handleDeleteRule = async (id: string) => {
    await deleteAlertRule(id)
    loadRules()
  }

  const handleTestChannel = async (id: string) => {
    setTestResult(prev => ({ ...prev, [id]: 'sending...' }))
    try {
      const res = await testNotificationChannel(id)
      setTestResult(prev => ({ ...prev, [id]: res.ok ? 'sent!' : (res.error || 'failed') }))
    } catch (e) {
      setTestResult(prev => ({ ...prev, [id]: String(e) }))
    }
    setTimeout(() => setTestResult(prev => { const n = { ...prev }; delete n[id]; return n }), 3000)
  }

  const handleDeleteChannel = async (id: string) => {
    await deleteNotificationChannel(id)
    loadChannels()
  }

  const handleToggleChannel = async (ch: NotificationChannel) => {
    await updateNotificationChannel(ch.id, { enabled: !ch.enabled })
    loadChannels()
  }

  return (
    <div>
      <div className="alerts-header">
        <h1 style={{ fontSize: '1.25rem', fontWeight: 600 }}>Alerts</h1>
        {activeAlerts.length > 0 && (
          <span className="alert-count-badge">{activeAlerts.length} active</span>
        )}
      </div>

      <div className="alert-tabs">
        <button className={`alert-tab ${tab === 'rules' ? 'active' : ''}`} onClick={() => setTab('rules')}>Rules</button>
        <button className={`alert-tab ${tab === 'channels' ? 'active' : ''}`} onClick={() => setTab('channels')}>Channels</button>
        <button className={`alert-tab ${tab === 'history' ? 'active' : ''}`} onClick={() => setTab('history')}>History</button>
      </div>

      {tab === 'rules' && (
        <div className="alert-panel">
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <span className="section-heading" style={{ margin: 0 }}>Alert Rules</span>
            <button className="btn primary" onClick={() => setShowCreateRule(true)}>Add Custom Rule</button>
          </div>

          {showCreateRule && <CreateRuleForm onDone={() => { setShowCreateRule(false); loadRules() }} />}

          <table className="node-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Field</th>
                <th>Condition</th>
                <th>Severity</th>
                <th>Enabled</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {rules.map(rule => (
                <tr key={rule.id} style={{ cursor: 'default' }}>
                  <td>
                    {rule.name}
                    {rule.well_known && <span className="well-known-badge" title="Built-in rule">built-in</span>}
                  </td>
                  <td>{rule.field}</td>
                  <td>
                    {OPERATOR_LABELS[rule.operator] || rule.operator}{' '}
                    {editingThreshold === rule.id ? (
                      <span className="inline-edit">
                        <input
                          type="text"
                          value={thresholdValue}
                          onChange={e => setThresholdValue(e.target.value)}
                          onKeyDown={e => e.key === 'Enter' && saveThreshold(rule.id)}
                          autoFocus
                          style={{ width: '6ch' }}
                        />
                        <button className="btn" onClick={() => saveThreshold(rule.id)} style={{ padding: '0.2rem 0.5rem', fontSize: '0.7rem' }}>Save</button>
                        <button className="btn" onClick={() => setEditingThreshold(null)} style={{ padding: '0.2rem 0.5rem', fontSize: '0.7rem' }}>Cancel</button>
                      </span>
                    ) : (
                      <span
                        className="editable-threshold"
                        onClick={() => { setEditingThreshold(rule.id); setThresholdValue(rule.threshold) }}
                        title="Click to edit"
                      >
                        {rule.threshold}
                      </span>
                    )}
                  </td>
                  <td>
                    <span className="severity-badge" style={{ color: SEVERITY_COLORS[rule.severity] || 'var(--text)' }}>
                      {rule.severity}
                    </span>
                  </td>
                  <td>
                    <label className="toggle-switch">
                      <input type="checkbox" checked={rule.enabled} onChange={() => toggleRule(rule)} />
                      <span className="toggle-slider" />
                    </label>
                  </td>
                  <td>
                    {!rule.well_known && (
                      <button className="btn danger" onClick={() => handleDeleteRule(rule.id)} style={{ padding: '0.25rem 0.5rem', fontSize: '0.7rem' }}>
                        Delete
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {tab === 'channels' && (
        <div className="alert-panel">
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <span className="section-heading" style={{ margin: 0 }}>Notification Channels</span>
            <button className="btn primary" onClick={() => setShowCreateChannel(true)}>Add Channel</button>
          </div>

          {showCreateChannel && <CreateChannelForm onDone={() => { setShowCreateChannel(false); loadChannels() }} />}

          {channels.length === 0 && (
            <p style={{ color: 'var(--text-dim)', fontSize: '0.875rem' }}>
              No notification channels configured. Add one to receive alerts via Telegram.
            </p>
          )}

          <div className="channels-grid">
            {channels.map(ch => (
              <div key={ch.id} className="channel-card">
                <div className="channel-card-header">
                  <strong>{ch.name}</strong>
                  <span className="channel-type-badge">{ch.channel_type}</span>
                </div>
                <div className="channel-card-status">
                  <label className="toggle-switch">
                    <input type="checkbox" checked={ch.enabled} onChange={() => handleToggleChannel(ch)} />
                    <span className="toggle-slider" />
                  </label>
                  <span style={{ fontSize: '0.75rem', color: ch.enabled ? 'var(--green)' : 'var(--text-dim)' }}>
                    {ch.enabled ? 'Enabled' : 'Disabled'}
                  </span>
                </div>
                <div className="channel-card-actions">
                  <button className="btn primary" onClick={() => handleTestChannel(ch.id)} style={{ fontSize: '0.75rem', padding: '0.3rem 0.6rem' }}>
                    {testResult[ch.id] || 'Test'}
                  </button>
                  <button className="btn danger" onClick={() => handleDeleteChannel(ch.id)} style={{ fontSize: '0.75rem', padding: '0.3rem 0.6rem' }}>
                    Delete
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {tab === 'history' && (
        <div className="alert-panel">
          <span className="section-heading">Alert History</span>
          <table className="node-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Node</th>
                <th>Rule</th>
                <th>Severity</th>
                <th>Status</th>
                <th>Value</th>
              </tr>
            </thead>
            <tbody>
              {history.map(h => (
                <tr key={h.id} style={{ cursor: 'default' }}>
                  <td>{formatTime(h.fired_at)}</td>
                  <td>{h.node_id}</td>
                  <td>{h.rule_name}</td>
                  <td>
                    <span className="severity-badge" style={{ color: SEVERITY_COLORS[h.severity] || 'var(--text)' }}>
                      {h.severity}
                    </span>
                  </td>
                  <td>
                    {h.resolved_at ? (
                      <span className="badge healthy">Resolved</span>
                    ) : (
                      <span className="badge offline">Firing</span>
                    )}
                  </td>
                  <td style={{ fontFamily: "'SF Mono', 'Fira Code', monospace" }}>{h.trigger_value}</td>
                </tr>
              ))}
              {history.length === 0 && (
                <tr>
                  <td colSpan={6} style={{ textAlign: 'center', color: 'var(--text-dim)', padding: '2rem' }}>
                    No alert history yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Create Rule Form
// ---------------------------------------------------------------------------

function CreateRuleForm({ onDone }: { onDone: () => void }) {
  const [name, setName] = useState('')
  const [field, setField] = useState('state')
  const [operator, setOperator] = useState('eq')
  const [threshold, setThreshold] = useState('')
  const [severity, setSeverity] = useState('warning')
  const [error, setError] = useState('')

  const handleSubmit = async () => {
    if (!name || !threshold) {
      setError('Name and threshold are required')
      return
    }
    try {
      await createAlertRule({ name, field, operator, threshold, severity })
      onDone()
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <div className="alert-form">
      <div className="form-grid" style={{ marginBottom: '0.75rem' }}>
        <div className="form-group">
          <label>Name</label>
          <input type="text" value={name} onChange={e => setName(e.target.value)} placeholder="My Custom Alert" />
        </div>
        <div className="form-group">
          <label>Field</label>
          <select value={field} onChange={e => setField(e.target.value)}>
            <option value="state">state</option>
            <option value="crash_looping">crash_looping</option>
            <option value="slots_behind">slots_behind</option>
            <option value="cpu_usage_percent">cpu_usage_percent</option>
            <option value="memory_percent">memory_percent</option>
            <option value="disk_percent">disk_percent</option>
            <option value="version_mismatch">version_mismatch</option>
            <option value="agent_uptime_secs">agent_uptime_secs</option>
            <option value="restart_count">restart_count</option>
            <option value="healthy">healthy</option>
          </select>
        </div>
        <div className="form-group">
          <label>Operator</label>
          <select value={operator} onChange={e => setOperator(e.target.value)}>
            <option value="eq">== (equals)</option>
            <option value="neq">!= (not equals)</option>
            <option value="gt">&gt; (greater than)</option>
            <option value="gte">&gt;= (greater or equal)</option>
            <option value="lt">&lt; (less than)</option>
            <option value="lte">&lt;= (less or equal)</option>
          </select>
        </div>
        <div className="form-group">
          <label>Threshold</label>
          <input type="text" value={threshold} onChange={e => setThreshold(e.target.value)} placeholder="e.g. 90 or off" />
        </div>
        <div className="form-group">
          <label>Severity</label>
          <select value={severity} onChange={e => setSeverity(e.target.value)}>
            <option value="critical">Critical</option>
            <option value="warning">Warning</option>
            <option value="info">Info</option>
          </select>
        </div>
      </div>
      {error && <p style={{ color: 'var(--red)', fontSize: '0.8rem', marginBottom: '0.5rem' }}>{error}</p>}
      <div style={{ display: 'flex', gap: '0.5rem' }}>
        <button className="btn primary" onClick={handleSubmit}>Create Rule</button>
        <button className="btn" onClick={onDone}>Cancel</button>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Create Channel Form
// ---------------------------------------------------------------------------

function CreateChannelForm({ onDone }: { onDone: () => void }) {
  const [name, setName] = useState('')
  const [botToken, setBotToken] = useState('')
  const [chatId, setChatId] = useState('')
  const [error, setError] = useState('')

  const handleSubmit = async () => {
    if (!name || !botToken || !chatId) {
      setError('All fields are required')
      return
    }
    try {
      const configJson = JSON.stringify({ bot_token: botToken, chat_id: chatId })
      await createNotificationChannel({ channel_type: 'telegram', name, config_json: configJson })
      onDone()
    } catch (e) {
      setError(String(e))
    }
  }

  return (
    <div className="alert-form">
      <div className="form-grid" style={{ marginBottom: '0.75rem' }}>
        <div className="form-group">
          <label>Channel Name</label>
          <input type="text" value={name} onChange={e => setName(e.target.value)} placeholder="e.g. Ops Telegram" />
        </div>
        <div className="form-group">
          <label>Bot Token</label>
          <input type="text" value={botToken} onChange={e => setBotToken(e.target.value)} placeholder="123456:ABC-DEF..." />
        </div>
        <div className="form-group">
          <label>Chat ID</label>
          <input type="text" value={chatId} onChange={e => setChatId(e.target.value)} placeholder="-1001234567890" />
        </div>
      </div>
      {error && <p style={{ color: 'var(--red)', fontSize: '0.8rem', marginBottom: '0.5rem' }}>{error}</p>}
      <div style={{ display: 'flex', gap: '0.5rem' }}>
        <button className="btn primary" onClick={handleSubmit}>Create Channel</button>
        <button className="btn" onClick={onDone}>Cancel</button>
      </div>
    </div>
  )
}

export default Alerts
