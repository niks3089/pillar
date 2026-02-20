import { useState, useEffect, FormEvent } from 'react'
import { Routes, Route, NavLink } from 'react-router-dom'
import Overview from './pages/Overview'
import NodeDetail from './pages/NodeDetail'
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
    <div className="app">
      <nav className="navbar">
        <NavLink to="/" className="nav-logo">Pillar</NavLink>
        <div className="nav-links">
          <NavLink to="/" end>Overview</NavLink>
          <a href="/grafana/d/pillar-fleet-overview" target="_blank" rel="noopener noreferrer">Grafana</a>
        </div>
        <div className="nav-user">
          <button className="btn nav-user-btn" onClick={() => setShowChangePassword(true)}>
            {username || 'admin'}
          </button>
          <button className="btn logout-btn" onClick={handleLogout}>Logout</button>
        </div>
      </nav>
      <UpdateBanner />
      <main className="content">
        <Routes>
          <Route path="/" element={<Overview />} />
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
    <div className="modal-overlay" onClick={onClose}>
      <form className="modal-card" onClick={e => e.stopPropagation()} onSubmit={handleSubmit}>
        <h2 className="modal-title">Change Credentials</h2>
        {error && <div className="login-error">{error}</div>}
        <label className="modal-label">Current Password</label>
        <input
          type="password"
          className="login-input"
          placeholder="Current password"
          value={currentPassword}
          onChange={e => setCurrentPassword(e.target.value)}
          autoComplete="current-password"
          autoFocus
          required
        />
        <label className="modal-label">New Username (optional)</label>
        <input
          type="text"
          className="login-input"
          placeholder="Leave blank to keep current"
          value={newUsername}
          onChange={e => setNewUsername(e.target.value)}
          autoComplete="username"
        />
        <label className="modal-label">New Password (optional)</label>
        <input
          type="password"
          className="login-input"
          placeholder="Leave blank to keep current"
          value={newPassword}
          onChange={e => setNewPassword(e.target.value)}
          autoComplete="new-password"
        />
        <label className="modal-label">Confirm New Password</label>
        <input
          type="password"
          className="login-input"
          placeholder="Confirm new password"
          value={confirmPassword}
          onChange={e => setConfirmPassword(e.target.value)}
          autoComplete="new-password"
          disabled={!newPassword}
        />
        <div className="modal-actions">
          <button type="button" className="btn" onClick={onClose}>Cancel</button>
          <button type="submit" className="btn primary" disabled={loading || !currentPassword}>
            {loading ? 'Saving...' : 'Save'}
          </button>
        </div>
      </form>
    </div>
  )
}

export default App
