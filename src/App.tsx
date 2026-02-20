import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import ForceGraph2D from 'react-force-graph-2d'
import './App.css'

interface Database {
  id: number
  name: string
  path: string
  relativePath: string
}

interface GraphNode {
  id: number
  name: string
  label: string
}

interface GraphLink {
  source: number
  target: number
  label: string
}

interface GraphData {
  nodes: GraphNode[]
  links: GraphLink[]
}

function App() {
  const [databases, setDatabases] = useState<Database[]>([])
  const [selectedId, setSelectedId] = useState(0)
  const [graphData, setGraphData] = useState<GraphData>({ nodes: [], links: [] })
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [darkMode, setDarkMode] = useState(true)
  const graphRef = useRef<any>(null)

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', darkMode ? 'dark' : 'light')
  }, [darkMode])

  useEffect(() => {
    fetch('http://localhost:3001/api/databases')
      .then(res => res.json())
      .then(setDatabases)
      .catch(err => setError(err.message))
  }, [])

  useEffect(() => {
    setLoading(true)
    setError(null)
    fetch(`http://localhost:3001/api/graph/${selectedId}`)
      .then(res => res.json())
      .then(data => {
        setGraphData(data)
        setLoading(false)
        setTimeout(() => {
          if (graphRef.current) {
            graphRef.current.zoomToFit(400)
          }
        }, 500)
      })
      .catch(err => {
        setError(err.message)
        setLoading(false)
      })
  }, [selectedId])

  const colorMap: Record<string, string> = useMemo(() => ({}), [])

  const nodeDegree = useMemo(() => {
    const degrees: Record<number, number> = {}
    graphData.nodes.forEach(n => degrees[n.id] = 0)
    graphData.links.forEach(link => {
      degrees[link.source] = (degrees[link.source] || 0) + 1
      degrees[link.target] = (degrees[link.target] || 0) + 1
    })
    return degrees
  }, [graphData])

  const maxDegree = useMemo(() => Math.max(1, ...Object.values(nodeDegree)), [nodeDegree])

  const getNodeColor = useCallback((label: string) => {
    if (!colorMap[label]) {
      const colors = ['#4e79a7', '#f28e2c', '#e15759', '#76b7b2', '#59a14f', '#edc949', '#af7aa1', '#ff9da7', '#9c755f', '#bab0ab']
      colorMap[label] = colors[Object.keys(colorMap).length % colors.length]
    }
    return colorMap[label]
  }, [])

  const getNodeSize = useCallback((node: GraphNode) => {
    const degree = nodeDegree[node.id] || 0
    return 4 + (degree / maxDegree) * 12
  }, [nodeDegree, maxDegree])

  const paintNode = useCallback((node: any, ctx: CanvasRenderingContext2D) => {
    const size = getNodeSize(node)
    const color = getNodeColor(node.label)

    ctx.fillStyle = color
    ctx.beginPath()
    ctx.arc(node.x, node.y, size, 0, 2 * Math.PI)
    ctx.fill()

    ctx.strokeStyle = darkMode ? '#222' : '#ddd'
    ctx.lineWidth = 1
    ctx.stroke()
  }, [getNodeSize, getNodeColor, darkMode])

  return (
    <div className="app-container">
      <button
        className="toggle-btn"
        onClick={() => setSidebarOpen(!sidebarOpen)}
      >
        {sidebarOpen ? '◀' : '▶'} Menu
      </button>

      <div className={`sidebar ${sidebarOpen ? '' : 'collapsed'}`}>
        <div className="sidebar-header">
          <h2 className="sidebar-title">Graphs</h2>
        </div>
          <div className="sidebar-content">
          {databases.length === 0 ? (
            <p style={{ color: 'var(--text-secondary)', padding: '16px' }}>No databases found</p>
          ) : (
            <ul className="file-list">
              {databases.map(db => (
                <li
                  key={db.id}
                  className={`file-item ${selectedId === db.id ? 'active' : ''}`}
                  onClick={() => setSelectedId(db.id)}
                  title={db.relativePath}
                >
                  {db.name}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', marginLeft: sidebarOpen ? 0 : 0 }}>
        <div className="header">
          <div className="header-left">
            <span className="graph-stats">
              {loading ? 'Loading...' : `${graphData.nodes.length} nodes, ${graphData.links.length} edges`}
            </span>
            {error && <span className="error-message">{error}</span>}
          </div>
          <div className="header-right">
            <button
              className="theme-toggle"
              onClick={() => setDarkMode(!darkMode)}
            >
              {darkMode ? '☀️ Light' : '🌙 Dark'}
            </button>
          </div>
        </div>

        <div className="graph-container">
          {!loading && !error && graphData.nodes.length > 0 && (
            <ForceGraph2D
              ref={graphRef}
              graphData={graphData}
              nodeCanvasObject={paintNode}
              nodeVal={getNodeSize}
              nodeLabel={(node) => `${node.label}: ${node.name}`}
              linkLabel={(link: any) => link.label}
              linkColor={() => darkMode ? 'rgba(100, 100, 100, 0.6)' : 'rgba(80, 80, 80, 0.6)'}
              linkWidth={1.5}
              linkDirectionalParticles={2}
              linkDirectionalParticleWidth={2}
              cooldownTicks={100}
              d3AlphaDecay={0.02}
              d3VelocityDecay={0.3}
              enableNodeDrag
            />
          )}
        </div>
      </div>
    </div>
  )
}

export default App
