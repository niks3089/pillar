import { Routes, Route, Link } from 'react-router-dom'
import Overview from './pages/Overview'
import NodeDetail from './pages/NodeDetail'

function App() {
  return (
    <div className="app">
      <nav className="navbar">
        <Link to="/" className="nav-logo">Pillar</Link>
        <div className="nav-links">
          <Link to="/">Overview</Link>
        </div>
      </nav>
      <main className="content">
        <Routes>
          <Route path="/" element={<Overview />} />
          <Route path="/nodes/:id" element={<NodeDetail />} />
        </Routes>
      </main>
    </div>
  )
}

export default App
