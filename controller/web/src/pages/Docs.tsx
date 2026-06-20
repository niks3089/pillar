import { useState, useMemo } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import opsDoc from '../operations.md?raw'

interface Section {
  id: string
  title: string
  body: string
}

function slug(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
}

// Split the operations guide into searchable sections by level-2 (## ) heading.
function parseSections(md: string): Section[] {
  const sections: Section[] = []
  let current: Section = { id: 'overview', title: 'Overview', body: '' }
  for (const line of md.split('\n')) {
    const m = line.match(/^##\s+(.+)/)
    if (m) {
      if (current.body.trim()) sections.push(current)
      current = { id: slug(m[1]), title: m[1].trim(), body: line + '\n' }
    } else {
      current.body += line + '\n'
    }
  }
  if (current.body.trim()) sections.push(current)
  return sections
}

export default function Docs() {
  const [query, setQuery] = useState('')
  const sections = useMemo(() => parseSections(opsDoc), [])
  const q = query.trim().toLowerCase()
  const filtered = q
    ? sections.filter(s => s.title.toLowerCase().includes(q) || s.body.toLowerCase().includes(q))
    : sections

  return (
    <div>
      <div className="node-header">
        <h1>Operations Guide</h1>
        <span className="meta">How to add, provision, upgrade, monitor &amp; alert</span>
      </div>

      <input
        type="text"
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder="Search the docs — e.g. upgrade, onboard, alert, firedancer, lagging..."
        autoFocus
        style={{
          width: '100%', background: 'var(--bg-input)', border: '1px solid var(--border)',
          borderRadius: 'var(--radius-sm)', padding: '0.7rem 1rem', color: 'var(--text)',
          fontSize: '0.9rem', outline: 'none', marginBottom: '1rem',
        }}
      />

      {/* Quick jump chips */}
      {!q && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: '0.4rem', marginBottom: '1.25rem' }}>
          {sections.map(s => (
            <a key={s.id} href={`#${s.id}`} className="badge registered" style={{ textDecoration: 'none' }}>
              {s.title}
            </a>
          ))}
        </div>
      )}

      {filtered.length === 0 && (
        <div className="config-panel" style={{ color: 'var(--text-dim)', textAlign: 'center' }}>
          No sections match “{query}”.
        </div>
      )}

      {filtered.map(s => (
        <div key={s.id} id={s.id} className="config-panel docs" style={{ marginBottom: '1rem' }}>
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{s.body}</ReactMarkdown>
        </div>
      ))}
    </div>
  )
}
