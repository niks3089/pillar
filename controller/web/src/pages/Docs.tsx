import { useState, useMemo, useEffect } from 'react'
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
  const [activeSection, setActiveSection] = useState('overview')
  const sections = useMemo(() => parseSections(opsDoc), [])
  const q = query.trim().toLowerCase()
  const filtered = q
    ? sections.filter(s => s.title.toLowerCase().includes(q) || s.body.toLowerCase().includes(q))
    : sections

  // Update active section on scroll
  useEffect(() => {
    const observer = new IntersectionObserver((entries) => {
      // Find the first intersecting entry
      for (const entry of entries) {
        if (entry.isIntersecting) {
          setActiveSection(entry.target.id)
        }
      }
    }, { rootMargin: '-100px 0px -60% 0px' })

    sections.forEach(s => {
      const el = document.getElementById(s.id)
      if (el) observer.observe(el)
    })

    return () => observer.disconnect()
  }, [sections])

  return (
    <div className="flex flex-col md:flex-row gap-12 w-full mx-auto pb-12 items-start relative">
      {/* Sidebar Navigation */}
      <aside className="w-full md:w-64 md:sticky top-28 shrink-0 flex flex-col gap-6">
        <div className="relative">
          <svg className="absolute left-3 top-2.5 w-4 h-4 text-zinc-500" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            type="text"
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Search docs (⌘K to open)"
            className="w-full pl-9 pr-4 py-2 bg-black/40 border border-white/10 rounded-md text-zinc-100 text-sm focus:outline-none focus:border-purple-500/50 transition-all placeholder:text-zinc-500 shadow-sm"
          />
        </div>

        <nav className="flex flex-col gap-1">
          <div className="text-xs font-semibold text-zinc-500 uppercase tracking-wider mb-2">Operations Guide</div>
          {sections.map(s => {
            const isActive = activeSection === s.id && !query
            return (
              <a 
                key={s.id} 
                href={`#${s.id}`} 
                className={`px-3 py-1.5 text-sm font-medium rounded-md transition-colors ${isActive ? 'bg-purple-500/10 text-purple-400' : 'text-zinc-400 hover:text-zinc-200 hover:bg-white/5'}`}
                onClick={() => setActiveSection(s.id)}
              >
                {s.title}
              </a>
            )
          })}
        </nav>
      </aside>

      {/* Main Content */}
      <main className="flex-1 min-w-0">
        <div className="mb-10">
           <div className="flex items-center gap-2 text-sm font-medium text-zinc-500 mb-6">
             <span>Documentation</span>
             <span>&rsaquo;</span>
             <span>Operations Guide</span>
             <span>&rsaquo;</span>
             <span className="text-zinc-300">{sections.find(s => s.id === activeSection)?.title || 'Overview'}</span>
           </div>
           
           <span className="inline-flex items-center gap-1.5 px-3 py-1 text-sm font-medium text-purple-400 bg-purple-500/10 border border-purple-500/20 rounded-md mb-8">
             Operations Guide
           </span>
        </div>

        {filtered.length === 0 && (
          <div className="bg-[#15131f] border border-white/10 rounded-xl p-8 text-center text-zinc-500 shadow-sm">
            No sections match “{query}”.
          </div>
        )}

        <div className="flex flex-col gap-12">
          {filtered.map(s => (
            <div key={s.id} id={s.id} className="prose prose-invert prose-zinc max-w-none prose-headings:text-zinc-100 prose-a:text-purple-400 hover:prose-a:text-purple-300 prose-pre:bg-[#15131f] prose-pre:border prose-pre:border-white/5 scroll-mt-32">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{s.body}</ReactMarkdown>
            </div>
          ))}
        </div>
      </main>
    </div>
  )
}
