import { useState, useEffect } from 'react'
import { fetchVersionInfo, upgradeController } from '../api'
import type { VersionInfo } from '../api'

function UpdateBanner() {
  const [info, setInfo] = useState<VersionInfo | null>(null)
  const [upgrading, setUpgrading] = useState(false)

  useEffect(() => {
    const check = () => {
      fetchVersionInfo().then(setInfo).catch(() => {})
    }
    check()
    const interval = setInterval(check, 60_000)
    return () => clearInterval(interval)
  }, [])

  if (!info?.controller_update) return null

  const update = info.controller_update

  const handleUpgrade = async () => {
    if (!confirm(`Upgrade controller to v${update.version}? The controller will restart.`)) return
    setUpgrading(true)
    try {
      await upgradeController()
      setTimeout(() => window.location.reload(), 5000)
    } catch (err) {
      alert(`Upgrade failed: ${err}`)
      setUpgrading(false)
    }
  }

  return (
    <div className="update-banner">
      <div className="update-banner-content">
        <span className="update-banner-text">
          Pillar <strong>v{update.version}</strong> is available
          {update.release_notes && <> &mdash; {update.release_notes}</>}
        </span>
        <button className="btn primary" onClick={handleUpgrade} disabled={upgrading}>
          {upgrading ? 'Upgrading...' : 'Upgrade Controller'}
        </button>
      </div>
    </div>
  )
}

export default UpdateBanner
