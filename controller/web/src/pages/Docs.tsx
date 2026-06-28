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
    <div className="flex flex-col gap-6 max-w-5xl mx-auto pb-12">
      <div className="flex flex-col md:flex-row md:items-center gap-4 bg-[#15131f] border border-white/10 rounded-xl p-6 shadow-sm">
        <h1 className="text-2xl font-semibold text-zinc-100 m-0 leading-none">Operations Guide</h1>
        <span className="text-sm font-medium text-zinc-500">How to add, provision, upgrade, monitor &amp; alert</span>
      </div>

      <input
        type="text"
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder="Search the docs — e.g. upgrade, onboard, alert, firedancer, lagging..."
        autoFocus
        className="w-full px-4 py-3 bg-black/40 border border-white/10 rounded-lg text-zinc-100 text-[15px] focus:outline-none focus:border-purple-500/50 focus:ring-1 focus:ring-purple-500/50 transition-all placeholder:text-zinc-600 shadow-sm"
      />

      {/* Quick jump chips */}
      {!q && (
        <div className="flex flex-wrap gap-2 mb-2">
          {sections.map(s => (
            <a key={s.id} href={`#${s.id}`} className="px-3 py-1.5 text-xs font-medium bg-zinc-800 text-zinc-300 hover:bg-zinc-700 hover:text-zinc-100 rounded-full border border-zinc-700 transition-colors">
              {s.title}
            </a>
          ))}
        </div>
      )}

      {filtered.length === 0 && (
        <div className="bg-[#15131f] border border-white/10 rounded-xl p-8 text-center text-zinc-500 shadow-sm">
          No sections match “{query}”.
        </div>
      )}

      <div className="flex flex-col gap-6">
        {filtered.map(s => (
          <div key={s.id} id={s.id} className="prose prose-invert prose-zinc max-w-none bg-[#15131f] border border-white/10 rounded-xl p-8 shadow-sm prose-headings:text-zinc-100 prose-a:text-purple-400 hover:prose-a:text-purple-300 prose-pre:bg-black/60 prose-pre:border prose-pre:border-white/5 scroll-mt-24">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{s.body}</ReactMarkdown>
          </div>
        ))}
      </div>
    </div>
  )
}
