import { Routes, Route, NavLink } from 'react-router-dom'
import Overview from './pages/Overview'
import NodeDetail from './pages/NodeDetail'
import Alerts from './pages/Alerts'
import UpdateBanner from './components/UpdateBanner'

function App() {
  return (
    <div className="app">
      <nav className="navbar">
        <NavLink to="/" className="nav-logo">Pillar</NavLink>
        <div className="nav-links">
          <NavLink to="/" end>Overview</NavLink>
          <NavLink to="/alerts">Alerts</NavLink>
          <a href="/grafana/d/pillar-fleet-overview" target="_blank" rel="noopener noreferrer">Grafana</a>
        </div>
      </nav>
      <UpdateBanner />
      <main className="content">
        <Routes>
          <Route path="/" element={<Overview />} />
          <Route path="/alerts" element={<Alerts />} />
          <Route path="/nodes/:id" element={<NodeDetail />} />
        </Routes>
      </main>
    </div>
  )
}

export default App
