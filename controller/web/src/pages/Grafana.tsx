import { useEffect, useState } from 'react'
import { fetchGrafanaSettings, saveGrafanaUrl } from '../api'

type Tab = 'fleet-overview' | 'node-detail'

function Grafana() {
  const [grafanaUrl, setGrafanaUrl] = useState('')
  const [urlInput, setUrlInput] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [copied, setCopied] = useState<string | null>(null)
  const [activeTab, setActiveTab] = useState<Tab>('fleet-overview')
  const [showSettings, setShowSettings] = useState(false)

  useEffect(() => {
    fetchGrafanaSettings()
      .then((s) => {
        setGrafanaUrl(s.grafana_url)
        setUrlInput(s.grafana_url)
      })
      .catch(() => {})
      .finally(() => setLoading(false))
  }, [])

  const handleSave = async () => {
    setSaving(true)
    setError('')
    try {
      const result = await saveGrafanaUrl(urlInput.trim())
      setGrafanaUrl(result.grafana_url)
      setShowSettings(false)
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Failed to save')
    } finally {
      setSaving(false)
    }
  }

  const copyDashboard = async (name: string) => {
    try {
      const res = await fetch(`/api/dashboards/${name}`)
      const text = await res.text()
      await navigator.clipboard.writeText(text)
      setCopied(name)
      setTimeout(() => setCopied(null), 2000)
    } catch {
      /* ignore */
    }
  }

  if (loading) return <div className="content"><p style={{ color: 'var(--text-dim)' }}>Loading...</p></div>

  // Embedded view
  if (grafanaUrl && !showSettings) {
    const dashboardUid = activeTab === 'fleet-overview' ? 'pillar-fleet-overview' : 'pillar-node-detail'
    const iframeSrc = `${grafanaUrl.replace(/\/$/, '')}/d/${dashboardUid}?orgId=1&kiosk`

    return (
      <div className="content grafana-fullwidth">
        <div className="grafana-toolbar">
          <div className="log-tabs" style={{ marginBottom: 0 }}>
            <button
              className={`log-tab ${activeTab === 'fleet-overview' ? 'active' : ''}`}
              onClick={() => setActiveTab('fleet-overview')}
            >
              Fleet Overview
            </button>
            <button
              className={`log-tab ${activeTab === 'node-detail' ? 'active' : ''}`}
              onClick={() => setActiveTab('node-detail')}
            >
              Node Detail
            </button>
          </div>
          <div style={{ display: 'flex', gap: '0.75rem', alignItems: 'center' }}>
            <a
              href={`${grafanaUrl.replace(/\/$/, '')}/d/${dashboardUid}`}
              target="_blank"
              rel="noopener noreferrer"
              className="btn"
            >
              Open in Grafana
            </a>
            <button className="btn" onClick={() => setShowSettings(true)}>
              Settings
            </button>
          </div>
        </div>
        <div className="grafana-iframe-container">
          <iframe
            src={iframeSrc}
            title={`Grafana - ${activeTab}`}
            style={{ width: '100%', height: '100%', border: 'none' }}
          />
        </div>
      </div>
    )
  }

  // Setup / settings view
  return (
    <div className="content">
      <h2 className="section-heading" style={{ marginBottom: '1.5rem' }}>
        {showSettings ? 'Grafana Settings' : 'Grafana Setup'}
      </h2>

      {!showSettings && (
        <div className="grafana-setup">
          <div className="grafana-step">
            <div className="grafana-step-number">1</div>
            <div className="grafana-step-content">
              <h3>Install Grafana</h3>
              <p>
                Install and start Grafana on your machine.{' '}
                <a
                  href="https://grafana.com/docs/grafana/latest/setup-grafana/installation/"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Installation docs
                </a>
              </p>
            </div>
          </div>

          <div className="grafana-step">
            <div className="grafana-step-number">2</div>
            <div className="grafana-step-content">
              <h3>Add Prometheus Data Source</h3>
              <p>
                In Grafana, go to <strong>Configuration &gt; Data Sources &gt; Add data source</strong> and select Prometheus. Set the URL to:
              </p>
              <code className="grafana-code-block">{window.location.origin}/metrics</code>
            </div>
          </div>

          <div className="grafana-step">
            <div className="grafana-step-number">3</div>
            <div className="grafana-step-content">
              <h3>Import Dashboards</h3>
              <p>
                Copy the dashboard JSON and import via <strong>Dashboards &gt; Import</strong> in Grafana.
              </p>
              <div style={{ display: 'flex', gap: '0.75rem', marginTop: '0.5rem' }}>
                <button className="btn primary" onClick={() => copyDashboard('fleet-overview')}>
                  {copied === 'fleet-overview' ? 'Copied!' : 'Copy Fleet Overview JSON'}
                </button>
                <button className="btn primary" onClick={() => copyDashboard('node-detail')}>
                  {copied === 'node-detail' ? 'Copied!' : 'Copy Node Detail JSON'}
                </button>
              </div>
            </div>
          </div>

          <div className="grafana-step">
            <div className="grafana-step-number">4</div>
            <div className="grafana-step-content">
              <h3>Enable Embedding</h3>
              <p>
                In <code>grafana.ini</code>, set <code>allow_embedding = true</code> under <code>[security]</code> to allow iframe embedding.
              </p>
            </div>
          </div>

          <div className="grafana-step">
            <div className="grafana-step-number">5</div>
            <div className="grafana-step-content">
              <h3>Enter Grafana URL</h3>
              <p>Paste your Grafana URL to embed dashboards in this page.</p>
            </div>
          </div>
        </div>
      )}

      <div style={{ marginTop: showSettings ? 0 : '1.5rem' }}>
        <div className="form-group" style={{ maxWidth: '500px' }}>
          <label>Grafana URL</label>
          <div style={{ display: 'flex', gap: '0.75rem' }}>
            <input
              type="text"
              value={urlInput}
              onChange={(e) => setUrlInput(e.target.value)}
              placeholder="https://grafana.example.com"
              style={{ flex: 1 }}
            />
            <button className="btn primary" onClick={handleSave} disabled={saving}>
              {saving ? 'Saving...' : 'Save'}
            </button>
            {showSettings && (
              <button className="btn" onClick={() => setShowSettings(false)}>
                Cancel
              </button>
            )}
          </div>
          {error && <p style={{ color: 'var(--red)', fontSize: '0.8125rem', marginTop: '0.25rem' }}>{error}</p>}
        </div>
      </div>
    </div>
  )
}

export default Grafana
