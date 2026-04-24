import { BrowserRouter, NavLink, Route, Routes } from 'react-router-dom';
import './design-system.css';
import './App.css';
import { BenchmarkPage } from './pages/BenchmarkPage';
import { CompilationDetailPage } from './pages/CompilationDetailPage';
import { CompilationsPage } from './pages/CompilationsPage';
import { ConflictsPage } from './pages/ConflictsPage';
import { EntitiesPage } from './pages/EntitiesPage';
import { EntityDetailPage } from './pages/EntityDetailPage';
import { HomePage } from './pages/HomePage';
import { IngestionDistributionPage } from './pages/IngestionDistributionPage';
import { MetricsPage } from './pages/MetricsPage';
import { PackDetailPage } from './pages/PackDetailPage';
import { ParserHealthPage } from './pages/ParserHealthPage';
import { PredicatesPage } from './pages/PredicatesPage';
import { RuntimePage } from './pages/RuntimePage';

function App() {
  return (
    <BrowserRouter>
      <div className="app-layout">
        <aside className="sidebar">
          <div className="sidebar-brand">
            <div className="loom-mark" aria-hidden="true" />
            <div>
              <div className="brand-name">loom</div>
              <div className="brand-sub">/memory</div>
            </div>
          </div>

          <nav>
            <div className="nav-section">Overview</div>
            <NavLink to="/" end>
              Pipeline Health
            </NavLink>
            <NavLink to="/runtime">Runtime</NavLink>

            <div className="nav-section">Knowledge</div>
            <NavLink to="/entities">Entities</NavLink>
            <NavLink to="/compilations">Compilations</NavLink>

            <div className="nav-section">Operations</div>
            <NavLink to="/conflicts">Conflicts</NavLink>
            <NavLink to="/predicates">Predicates</NavLink>

            <div className="nav-section">Insights</div>
            <NavLink to="/metrics">Metrics</NavLink>
            <NavLink to="/benchmarks">Benchmarks</NavLink>

            <div className="nav-section">Ingestion</div>
            <NavLink to="/ingestion/distribution">Distribution</NavLink>
            <NavLink to="/ingestion/parsers">Parser Health</NavLink>
          </nav>

          <div className="sidebar-footer">local</div>
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
            <Route path="/ingestion/parsers" element={<ParserHealthPage />} />
            <Route path="/ingestion/distribution" element={<IngestionDistributionPage />} />
            <Route path="/runtime" element={<RuntimePage />} />
          </Routes>
        </main>
      </div>
    </BrowserRouter>
  );
}

export default App;
