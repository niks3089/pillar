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
    <div className="bg-purple-900/40 border-b border-purple-500/30 w-full">
      <div className="max-w-7xl mx-auto px-4 py-3 flex flex-col md:flex-row items-center justify-between gap-4">
        <span className="text-sm text-purple-200">
          Pillar <strong>v{update.version}</strong> is available
          {update.release_notes && <> &mdash; {update.release_notes}</>}
        </span>
        <button className="px-4 py-1.5 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all whitespace-nowrap" onClick={handleUpgrade} disabled={upgrading}>
          {upgrading ? 'Upgrading...' : 'Upgrade Controller'}
        </button>
      </div>
    </div>
  )
}

export default UpdateBanner
