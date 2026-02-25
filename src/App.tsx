import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import ForceGraph2D from 'react-force-graph-2d'
import type { NodeObject } from 'react-force-graph-2d'
import './App.css'

interface Database {
  id: number
  name: string
  path: string
  relativePath: string
}

interface GraphNode {
  id: string
  name: string
  label: string
}

interface GraphLink {
  source: string
  target: string
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
  const [customQuery, setCustomQuery] = useState<string>('')
  const [isCustomQuery, setIsCustomQuery] = useState(false)
  const [queryActivated, setQueryActivated] = useState(false)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const graphRef = useRef<any>(null)
  const customQueryRef = useRef<string>('')
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const fetchDatabases = () => {
    invoke<Database[]>('get_databases')
      .then(setDatabases)
      .catch(err => setError(String(err)))
  }

  const fetchDirectories = (dir: string) => {
    setPickerError(null)
    invoke<{ current: string; parent: string; directories: { name: string; path: string; type: string }[]; files: { name: string; path: string; type: string }[] }>('get_directories', { path: dir || null })
      .then(data => {
        setCurrentDir(data.current || dir || '')
        setParentDir(data.parent || '')
        setDirs(data.directories || [])
        setFiles(data.files || [])
      })
      .catch(err => {
        setPickerError(String(err))
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

  const fetchGraphData = useCallback(() => {
    if (databases.length === 0) {
      setGraphData({ nodes: [], links: [] })
      return
    }
    setLoading(true)
    setError(null)

    const query = customQueryRef.current.trim()
    if (query) {
      invoke<GraphData>('execute_query', { id: selectedId, query })
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
          setError(String(err))
          setLoading(false)
        })
    } else {
      invoke<GraphData>('get_graph', { id: selectedId })
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
          setError(String(err))
          setLoading(false)
        })
    }
  }, [selectedId, databases.length])

  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    fetchGraphData()
  }, [fetchGraphData])
  /* eslint-enable react-hooks/set-state-in-effect */

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
      await invoke('add_database', { filePath })
      fetchDatabases()
      setFilePickerOpen(false)
      setPickerError(null)
      setManualPath('')
    } catch (err) {
      setPickerError(String(err))
    }
  }

  const colorMapRef = useRef<Record<string, string>>({})
  const edgeColorMapRef = useRef<Record<string, string>>({})

  const nodeDegree = useMemo(() => {
    const degrees: Record<string, number> = {}
    graphData.nodes.forEach(n => degrees[n.id] = 0)
    graphData.links.forEach(link => {
      const src = typeof link.source === 'object' ? (link.source as NodeObject).id : link.source
      const dst = typeof link.target === 'object' ? (link.target as NodeObject).id : link.target
      degrees[src as string] = (degrees[src as string] || 0) + 1
      degrees[dst as string] = (degrees[dst as string] || 0) + 1
    })
    return degrees
  }, [graphData])

  const maxDegree = useMemo(() => Math.max(1, ...Object.values(nodeDegree)), [nodeDegree])

  const getNodeColor = useCallback((label: string) => {
    if (!colorMapRef.current[label]) {
      const colors = ['#4e79a7', '#f28e2c', '#e15759', '#76b7b2', '#59a14f', '#edc949', '#af7aa1', '#ff9da7', '#9c755f', '#bab0ab']
      colorMapRef.current[label] = colors[Object.keys(colorMapRef.current).length % colors.length]
    }
    return colorMapRef.current[label]
  }, [])

  const getEdgeColor = useCallback((label: string) => {
    if (!edgeColorMapRef.current[label]) {
      const colors = ['#5a9bd5', '#e07b39', '#d94452', '#6cc4a4', '#8cc63f', '#f0c040', '#c47ab6', '#ff7f7f', '#b8860b', '#7b9ea8']
      edgeColorMapRef.current[label] = colors[Object.keys(edgeColorMapRef.current).length % colors.length]
    }
    return edgeColorMapRef.current[label]
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

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
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
      const measured = ctx.measureText(label)
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
              nodeVal={(node) => { const s = getNodeSize(node); return s * s; }}
              nodeRelSize={1}
              nodeLabel={(node) => `${node.label}: ${node.name}`}
              linkLabel={(link) => link.label}
              linkColor={(link) => getEdgeColor(link.label)}
              linkWidth={2.5}
              linkDirectionalArrowLength={6}
              linkDirectionalArrowRelPos={1}
              linkDirectionalArrowColor={(link) => getEdgeColor(link.label)}
              linkDirectionalParticles={2}
              linkDirectionalParticleWidth={2}
              cooldownTicks={100}
              d3AlphaDecay={0.02}
              d3VelocityDecay={0.3}
              enableNodeDrag
            />
          )}
        </div>

        <div className="query-box">
          <textarea
            value={customQuery}
            placeholder="Enter Cypher query (e.g., MATCH (n) RETURN n LIMIT 100)"
            onChange={e => {
              const val = e.target.value
              setCustomQuery(val)
              customQueryRef.current = val
              setIsCustomQuery(val.trim().length > 0)
              // After first activation, debounce auto-execution
              if (queryActivated && val.trim()) {
                if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current)
                debounceTimerRef.current = setTimeout(() => {
                  fetchGraphData()
                }, 3000)
              }
            }}
            onKeyDown={e => {
              if (e.key === 'Enter' && !e.shiftKey && customQuery.trim()) {
                e.preventDefault()
                if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current)
                setQueryActivated(true)
                fetchGraphData()
              }
            }}
            className="query-input"
            rows={5}
          />
          <div className="query-actions">
            <button
              className="query-btn"
              onClick={() => {
                if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current)
                setQueryActivated(true)
                fetchGraphData()
              }}
              disabled={!customQuery.trim()}
            >
              Run
            </button>
            {isCustomQuery && (
              <button
                className="query-btn secondary"
                onClick={() => {
                  if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current)
                  setCustomQuery('')
                  customQueryRef.current = ''
                  setIsCustomQuery(false)
                  setQueryActivated(false)
                }}
              >
                Reset
              </button>
            )}
          </div>
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
