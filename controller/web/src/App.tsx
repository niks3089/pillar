import { Routes, Route, NavLink } from 'react-router-dom'
import Overview from './pages/Overview'
import NodeDetail from './pages/NodeDetail'
import Grafana from './pages/Grafana'

function App() {
  return (
    <div className="app">
      <nav className="navbar">
        <NavLink to="/" className="nav-logo">Pillar</NavLink>
        <div className="nav-links">
          <NavLink to="/" end>Overview</NavLink>
          <NavLink to="/grafana">Grafana</NavLink>
        </div>
      </nav>
      <main className="content">
        <Routes>
          <Route path="/" element={<Overview />} />
          <Route path="/nodes/:id" element={<NodeDetail />} />
          <Route path="/grafana" element={<Grafana />} />
        </Routes>
      </main>
    </div>
  )
}

export default App
