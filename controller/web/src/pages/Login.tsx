import { useState, FormEvent } from 'react'

interface LoginProps {
  onLogin: () => void
}

export default function Login({ onLogin }: LoginProps) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    setError('')
    setLoading(true)

    try {
      const res = await fetch('/api/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username, password }),
      })

      if (res.ok) {
        onLogin()
      } else {
        const data = await res.json().catch(() => null)
        setError(data?.error || 'Authentication failed')
      }
    } catch {
      setError('Network error')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-[#0a0911] px-4 selection:bg-purple-500/30 font-sans text-zinc-100">
      <form className="w-full max-w-[380px] p-8 bg-[#15131f] border border-white/10 rounded-2xl shadow-2xl flex flex-col items-center" onSubmit={handleSubmit}>
        <img src="/pillar-logo.png" alt="Pillar" className="h-10 mb-4" />
        <h1 className="text-xl font-semibold mb-1">Welcome back</h1>
        <p className="text-sm text-zinc-400 mb-8">Sign in to your Pillar dashboard</p>
        
        {error && <div className="w-full p-3 mb-6 text-sm text-red-400 bg-red-950/30 border border-red-900/50 rounded-md text-center">{error}</div>}
        
        <div className="w-full flex flex-col gap-4 mb-8">
          <div className="flex flex-col gap-1.5">
            <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Username</label>
            <input
              type="text"
              className="w-full px-3 py-2.5 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600"
              placeholder="admin"
              value={username}
              onChange={e => setUsername(e.target.value)}
              autoComplete="username"
              autoFocus
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <label className="text-xs font-medium text-zinc-400 uppercase tracking-wider">Password</label>
            <input
              type="password"
              className="w-full px-3 py-2.5 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600"
              placeholder="••••••••"
              value={password}
              onChange={e => setPassword(e.target.value)}
              autoComplete="current-password"
            />
          </div>
        </div>
        
        <button type="submit" className="w-full py-2.5 text-sm font-medium text-white bg-purple-600 hover:bg-purple-500 rounded-md border border-purple-500/50 shadow-sm transition-all disabled:opacity-50 disabled:cursor-not-allowed" disabled={loading || !username || !password}>
          {loading ? 'Signing in...' : 'Sign in'}
        </button>
      </form>
    </div>
  )
}
