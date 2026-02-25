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
  const [filePickerOpen, setFilePickerOpen] = useState(false)
  const [currentDir, setCurrentDir] = useState<string>('')
  const [dirs, setDirs] = useState<{ name: string; path: string; type: string }[]>([])
  const [files, setFiles] = useState<{ name: string; path: string; type: string }[]>([])
  const [parentDir, setParentDir] = useState<string>('')
  const [manualPath, setManualPath] = useState<string>('')
  const [pickerError, setPickerError] = useState<string | null>(null)
  const graphRef = useRef<any>(null)

  const fetchDatabases = () => {
    fetch('http://localhost:3001/api/databases')
      .then(res => res.json())
      .then(setDatabases)
      .catch(err => setError(err.message))
  }

  const fetchDirectories = (dir: string) => {
    setPickerError(null)
    fetch(`http://localhost:3001/api/directories?path=${encodeURIComponent(dir)}`)
      .then(res => {
        if (!res.ok) {
          return res.json().then(err => { throw new Error(err.error || 'Failed to fetch directories') })
        }
        return res.json()
      })
      .then(data => {
        setCurrentDir(data.current || dir || '')
        setParentDir(data.parent || '')
        setDirs(data.directories || [])
        setFiles(data.files || [])
      })
      .catch(err => {
        setPickerError(err.message)
        setCurrentDir(dir || 'Failed to load')
        setDirs([])
        setFiles([])
      })
  }

  useEffect(() => {
    fetchDatabases()
  }, [])

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', darkMode ? 'dark' : 'light')
  }, [darkMode])

  useEffect(() => {
    if (databases.length === 0) {
      setGraphData({ nodes: [], links: [] })
      return
    }
    setLoading(true)
    setError(null)
    fetch(`http://localhost:3001/api/graph/${selectedId}`)
      .then(res => {
        if (!res.ok) {
          return res.json().then(err => Promise.reject(new Error(err.error)))
        }
        return res.json()
      })
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
  }, [selectedId, databases.length])

  const openFilePicker = () => {
    setManualPath('')
    setPickerError(null)
    fetchDirectories('')
    setFilePickerOpen(true)
  }

  const navigateToDir = (dir: string) => {
    fetchDirectories(dir)
  }

  const addDatabase = async (filePath: string) => {
    try {
      const res = await fetch('http://localhost:3001/api/databases', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ filePath })
      })
      if (res.ok) {
        fetchDatabases()
        setFilePickerOpen(false)
        setPickerError(null)
        setManualPath('')
      } else {
        const err = await res.json()
        setPickerError(err.error || 'Failed to add database')
      }
    } catch (err: any) {
      setPickerError(err.message || 'Network error')
    }
  }

  const colorMap: Record<string, string> = useMemo(() => ({}), [])
  const edgeColorMap: Record<string, string> = useMemo(() => ({}), [])

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

  const getEdgeColor = useCallback((label: string) => {
    if (!edgeColorMap[label]) {
      const colors = ['#5a9bd5', '#e07b39', '#d94452', '#6cc4a4', '#8cc63f', '#f0c040', '#c47ab6', '#ff7f7f', '#b8860b', '#7b9ea8']
      edgeColorMap[label] = colors[Object.keys(edgeColorMap).length % colors.length]
    }
    return edgeColorMap[label]
  }, [])

  const getNodeSize = useCallback((node: GraphNode) => {
    const degree = nodeDegree[node.id] || 0
    return 4 + (degree / maxDegree) * 12
  }, [nodeDegree, maxDegree])

  const labelSizeThreshold = useMemo(() => {
    const sizes = graphData.nodes.map(n => {
      const degree = nodeDegree[n.id] || 0
      return 4 + (degree / maxDegree) * 12
    })
    sizes.sort((a, b) => b - a)
    // Label the top 20% of nodes, but at least the top 5
    const cutoffIndex = Math.max(4, Math.floor(sizes.length * 0.2) - 1)
    return sizes[Math.min(cutoffIndex, sizes.length - 1)] ?? 16
  }, [graphData.nodes, nodeDegree, maxDegree])

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

    if (size >= labelSizeThreshold && node.name) {
      const fontSize = 3
      ctx.font = `${fontSize}px Sans-Serif`
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillStyle = '#fff'

      const maxWidth = size * 1.6
      let label = node.name
      let measured = ctx.measureText(label)
      if (measured.width > maxWidth) {
        while (label.length > 1 && ctx.measureText(label + '\u2026').width > maxWidth) {
          label = label.slice(0, -1)
        }
        label = label + '\u2026'
      }
      ctx.fillText(label, node.x, node.y)
    }
  }, [getNodeSize, getNodeColor, darkMode, labelSizeThreshold])

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
          <button className="add-db-btn" onClick={openFilePicker}>+ Add</button>
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
              nodeVal={(node: any) => { const s = getNodeSize(node); return s * s; }}
              nodeRelSize={1}
              nodeLabel={(node) => `${node.label}: ${node.name}`}
              linkLabel={(link: any) => link.label}
              linkColor={(link: any) => getEdgeColor(link.label)}
              linkWidth={2.5}
              linkDirectionalArrowLength={6}
              linkDirectionalArrowRelPos={1}
              linkDirectionalArrowColor={(link: any) => getEdgeColor(link.label)}
              linkDirectionalParticles={2}
              linkDirectionalParticleWidth={2}
              cooldownTicks={100}
              d3AlphaDecay={0.02}
              d3VelocityDecay={0.3}
              enableNodeDrag
            />
          )}
        </div>

        {filePickerOpen && (
          <div className="modal-overlay" onClick={() => setFilePickerOpen(false)}>
            <div className="file-picker-modal" onClick={e => e.stopPropagation()}>
              <div className="modal-header">
                <h3>Add Database</h3>
                <button className="close-btn" onClick={() => setFilePickerOpen(false)}>×</button>
              </div>
              <div className="modal-path">
                <button onClick={() => navigateToDir(parentDir)} disabled={!parentDir || parentDir === currentDir}>↑ Up</button>
                <span className="current-path">{currentDir || 'Loading...'}</span>
              </div>
              {pickerError ? (
                <div style={{ padding: '16px', color: '#ff6b6b', backgroundColor: 'rgba(255, 107, 107, 0.15)', borderBottom: '1px solid var(--border-color)', textAlign: 'center' }}>
                  <strong>Error:</strong> {pickerError}
                </div>
              ) : (
                <div className="dir-list">
                  {(dirs || []).map(dir => (
                    <div key={dir.path} className="dir-item" onClick={() => navigateToDir(dir.path)}>
                      📁 {dir.name}
                    </div>
                  ))}
                  {(files || []).map(file => (
                    <div key={file.path} className="file-item" onClick={() => addDatabase(file.path)}>
                      🗄️ {file.name}
                    </div>
                  ))}
                  {(!dirs || dirs.length === 0) && (!files || files.length === 0) && <p style={{ color: 'var(--text-secondary)', padding: '8px' }}>No items</p>}
                </div>
              )}
              <div className="modal-footer">
                <input
                  type="text"
                  value={manualPath}
                  placeholder="Enter full path to .lbdb file..."
                  onChange={e => setManualPath(e.target.value)}
                  onKeyDown={e => {
                    if (e.key === 'Enter' && manualPath.trim()) {
                      addDatabase(manualPath.trim())
                    }
                  }}
                  style={{ borderColor: pickerError ? '#ff6b6b' : undefined }}
                />
                {pickerError && manualPath && (
                  <div style={{ marginTop: '8px', color: '#ff6b6b', fontSize: '13px' }}>
                    {pickerError}
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

export default App
