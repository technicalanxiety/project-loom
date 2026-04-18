import { BrowserRouter, Routes, Route } from "react-router-dom";

function App() {
  return (
    <BrowserRouter>
      <div style={{ fontFamily: "system-ui, sans-serif", padding: "2rem" }}>
        <h1>Loom Dashboard</h1>
        <p>Pipeline health, knowledge graph, compilation traces, and more.</p>
        <Routes>
          <Route path="/" element={<Home />} />
          {/* TODO: Add routes for each dashboard view */}
        </Routes>
      </div>
    </BrowserRouter>
  );
}

function Home() {
  return (
    <div>
      <h2>Pipeline Health</h2>
      <p>TODO: Implement pipeline health dashboard</p>
    </div>
  );
}

export default App;
