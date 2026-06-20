import { useEffect, useState } from 'react'
import { fetchGrafanaSettings, saveGrafanaUrl } from '../api'

type Tab = 'fleet-overview' | 'node-detail'

function Grafana() {
  const [grafanaUrl, setGrafanaUrl] = useState('')
  const [urlInput, setUrlInput] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
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

  if (loading) return <div className="content"><p style={{ color: 'var(--text-dim)' }}>Loading...</p></div>

  // Embedded view — iframe uses the controller's reverse proxy at /grafana/
  if (grafanaUrl && !showSettings) {
    const dashboardUid = activeTab === 'fleet-overview' ? 'pillar-fleet-overview' : 'pillar-node-detail'
    const iframeSrc = `/grafana/d/${dashboardUid}?orgId=1&kiosk`

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
              href={`/grafana/d/${dashboardUid}`}
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

  // Not configured / settings view
  return (
    <div className="content">
      <div className="grafana-not-configured">
        <h2 className="section-heading">
          {showSettings ? 'Grafana Settings' : 'Grafana'}
        </h2>
        <p style={{ color: 'var(--text-dim)', fontSize: '0.875rem', margin: '0.5rem 0 1.5rem' }}>
          {grafanaUrl
            ? 'Update the local Grafana URL that the controller proxies to.'
            : 'Enter the local Grafana URL (e.g. http://localhost:3000). The controller reverse-proxies it so dashboards are accessible remotely.'}
        </p>
        <div className="form-group" style={{ maxWidth: '500px' }}>
          <label>Local Grafana URL</label>
          <div style={{ display: 'flex', gap: '0.75rem' }}>
            <input
              type="text"
              value={urlInput}
              onChange={(e) => setUrlInput(e.target.value)}
              placeholder="http://localhost:3000"
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
