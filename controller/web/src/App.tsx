import { useState, useEffect, FormEvent } from 'react'
import { Routes, Route, NavLink } from 'react-router-dom'
import Overview from './pages/Overview'
import NodeDetail from './pages/NodeDetail'
import Docs from './pages/Docs'
import Login from './pages/Login'
import UpdateBanner from './components/UpdateBanner'
import { checkAuth, logout, changeCredentials } from './auth'

function App() {
  const [authed, setAuthed] = useState<boolean | null>(null)
  const [username, setUsername] = useState('')
  const [showChangePassword, setShowChangePassword] = useState(false)

  useEffect(() => {
    checkAuth().then(result => {
      setAuthed(result.authenticated)
      setUsername(result.username)
    })
  }, [])

  // Loading state
  if (authed === null) return null

  if (!authed) {
    return <Login onLogin={() => {
      checkAuth().then(result => {
        setAuthed(result.authenticated)
        setUsername(result.username)
      })
    }} />
  }

  async function handleLogout() {
    await logout()
    setAuthed(false)
    setUsername('')
  }

  return (
    <div className="min-h-screen bg-[#0a0911] text-zinc-100 font-sans selection:bg-purple-500/30">
      <nav className="sticky top-0 z-50 flex items-center justify-between px-6 py-4 bg-[#0a0911]/80 backdrop-blur-md border-b border-white/5">
        <div className="flex items-center gap-8">
          <NavLink to="/" className="flex items-center gap-2 hover:opacity-80 transition-opacity">
            <img src="/pillar-logo.png" alt="Pillar" className="h-6 w-auto" />
          </NavLink>
          <div className="flex items-center gap-6 text-sm font-medium text-zinc-400">
            <NavLink to="/" end className={({isActive}) => isActive ? "text-zinc-100" : "hover:text-zinc-100 transition-colors"}>Overview</NavLink>
            <a href="/grafana/d/pillar-fleet-overview" target="_blank" rel="noopener noreferrer" className="hover:text-zinc-100 transition-colors">Metrics</a>
            <NavLink to="/docs" className={({isActive}) => isActive ? "text-zinc-100" : "hover:text-zinc-100 transition-colors"}>Docs</NavLink>
          </div>
        </div>
        <div className="flex items-center gap-3">
          <button className="px-3 py-1.5 text-sm font-medium rounded-md bg-white/5 border border-white/10 hover:bg-white/10 transition-colors text-zinc-300 hover:text-white" onClick={() => setShowChangePassword(true)}>
            {username || 'admin'}
          </button>
          <button className="px-3 py-1.5 text-sm font-medium rounded-md bg-transparent border border-transparent hover:bg-white/5 transition-colors text-zinc-400 hover:text-zinc-300" onClick={handleLogout}>Logout</button>
        </div>
      </nav>
      <UpdateBanner />
      <main className="max-w-7xl mx-auto px-6 py-8">
        <Routes>
          <Route path="/" element={<Overview />} />
          <Route path="/docs" element={<Docs />} />
          <Route path="/nodes/:id" element={<NodeDetail />} />
        </Routes>
      </main>
      {showChangePassword && (
        <ChangePasswordModal
          onClose={() => setShowChangePassword(false)}
          onChanged={() => {
            setShowChangePassword(false)
            handleLogout()
          }}
        />
      )}
    </div>
  )
}

function ChangePasswordModal({ onClose, onChanged }: { onClose: () => void; onChanged: () => void }) {
  const [currentPassword, setCurrentPassword] = useState('')
  const [newUsername, setNewUsername] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError('')

    if (newPassword && newPassword !== confirmPassword) {
      setError('New passwords do not match')
      return
    }

    if (!newUsername && !newPassword) {
      setError('Provide a new username or password')
      return
    }

    setLoading(true)
    const result = await changeCredentials(currentPassword, newUsername, newPassword)
    setLoading(false)

    if (result.ok) {
      onChanged()
    } else {
      setError(result.error || 'Failed to update credentials')
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={onClose}>
      <form className="w-full max-w-md p-6 bg-[#15131f] border border-white/10 rounded-xl shadow-2xl flex flex-col gap-4" onClick={e => e.stopPropagation()} onSubmit={handleSubmit}>
        <h2 className="text-xl font-semibold text-zinc-100 mb-2">Change Credentials</h2>
        {error && <div className="p-3 text-sm text-red-400 bg-red-950/30 border border-red-900/50 rounded-md">{error}</div>}
        
        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-zinc-400">Current Password</label>
          <input
            type="password"
            className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600"
            placeholder="Current password"
            value={currentPassword}
            onChange={e => setCurrentPassword(e.target.value)}
            autoComplete="current-password"
            autoFocus
            required
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-zinc-400">New Username (optional)</label>
          <input
            type="text"
            className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600"
            placeholder="Leave blank to keep current"
            value={newUsername}
            onChange={e => setNewUsername(e.target.value)}
            autoComplete="username"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-zinc-400">New Password (optional)</label>
          <input
            type="password"
            className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600"
            placeholder="Leave blank to keep current"
            value={newPassword}
            onChange={e => setNewPassword(e.target.value)}
            autoComplete="new-password"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label className="text-sm font-medium text-zinc-400">Confirm New Password</label>
          <input
            type="password"
            className="w-full px-3 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600"
            placeholder="Confirm new password"
            value={confirmPassword}
            onChange={e => setConfirmPassword(e.target.value)}
            autoComplete="new-password"
            disabled={!newPassword}
          />
        </div>

        <div className="flex items-center justify-end gap-3 mt-4">
          <button type="button" className="px-4 py-2 text-sm font-medium text-zinc-400 hover:text-zinc-200 transition-colors" onClick={onClose}>Cancel</button>
          <button type="submit" className="px-4 py-2 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all disabled:opacity-50 disabled:cursor-not-allowed" disabled={loading || !currentPassword}>
            {loading ? 'Saving...' : 'Save'}
          </button>
        </div>
      </form>
    </div>
  )
}

export default App
