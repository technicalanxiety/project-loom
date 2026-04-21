import { BrowserRouter, NavLink, Route, Routes } from 'react-router-dom';
import './App.css';
import { BenchmarkPage } from './pages/BenchmarkPage';
import { CompilationDetailPage } from './pages/CompilationDetailPage';
import { CompilationsPage } from './pages/CompilationsPage';
import { ConflictsPage } from './pages/ConflictsPage';
import { EntitiesPage } from './pages/EntitiesPage';
import { EntityDetailPage } from './pages/EntityDetailPage';
import { HomePage } from './pages/HomePage';
import { MetricsPage } from './pages/MetricsPage';
import { PackDetailPage } from './pages/PackDetailPage';
import { PredicatesPage } from './pages/PredicatesPage';

/** Root application component with sidebar navigation and route definitions. */
function App() {
  return (
    <BrowserRouter>
      <div className="app-layout">
        <aside className="sidebar">
          <div className="sidebar-brand">
            <h1>Loom</h1>
            <p>Memory Compiler</p>
          </div>
          <nav>
            <div className="nav-section">Overview</div>
            <NavLink to="/" end>
              Pipeline Health
            </NavLink>

            <div className="nav-section">Knowledge</div>
            <NavLink to="/entities">Entities</NavLink>
            <NavLink to="/compilations">Compilations</NavLink>

            <div className="nav-section">Operations</div>
            <NavLink to="/conflicts">Conflicts</NavLink>
            <NavLink to="/predicates">Predicates</NavLink>

            <div className="nav-section">Insights</div>
            <NavLink to="/metrics">Metrics</NavLink>
            <NavLink to="/benchmarks">Benchmarks</NavLink>
          </nav>
        </aside>

        <main className="main-content">
          <Routes>
            <Route path="/" element={<HomePage />} />
            <Route path="/compilations" element={<CompilationsPage />} />
            <Route path="/compilations/:id" element={<CompilationDetailPage />} />
            <Route path="/entities" element={<EntitiesPage />} />
            <Route path="/entities/:id" element={<EntityDetailPage />} />
            <Route path="/conflicts" element={<ConflictsPage />} />
            <Route path="/predicates" element={<PredicatesPage />} />
            <Route path="/predicates/packs/:pack" element={<PackDetailPage />} />
            <Route path="/metrics" element={<MetricsPage />} />
            <Route path="/benchmarks" element={<BenchmarkPage />} />
          </Routes>
        </main>
      </div>
    </BrowserRouter>
  );
}

export default App;
