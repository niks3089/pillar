export async function checkAuth(): Promise<{ authenticated: boolean; username: string }> {
  try {
    const res = await fetch('/api/auth/check')
    if (!res.ok) return { authenticated: false, username: '' }
    const data = await res.json()
    return { authenticated: data.authenticated === true, username: data.username || '' }
  } catch {
    return { authenticated: false, username: '' }
  }
}

export async function login(username: string, password: string): Promise<{ ok: boolean; error?: string }> {
  try {
    const res = await fetch('/api/login', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password }),
    })
    if (res.ok) return { ok: true }
    const data = await res.json().catch(() => null)
    return { ok: false, error: data?.error || 'Invalid credentials' }
  } catch {
    return { ok: false, error: 'Network error' }
  }
}

export async function logout(): Promise<void> {
  await fetch('/api/logout', { method: 'POST' }).catch(() => {})
}

export async function changeCredentials(
  currentPassword: string,
  newUsername: string,
  newPassword: string,
): Promise<{ ok: boolean; error?: string }> {
  try {
    const body: Record<string, string> = { current_password: currentPassword }
    if (newUsername) body.new_username = newUsername
    if (newPassword) body.new_password = newPassword
    const res = await fetch('/api/auth/credentials', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
    if (res.ok) return { ok: true }
    const data = await res.json().catch(() => null)
    return { ok: false, error: data?.error || 'Failed to update credentials' }
  } catch {
    return { ok: false, error: 'Network error' }
  }
}
