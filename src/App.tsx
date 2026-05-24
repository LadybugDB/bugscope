import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import ForceGraph2D from 'react-force-graph-2d'
import type { GraphData as ForceGraphData, NodeObject } from 'react-force-graph-2d'
import { Sigma } from './vendor/sigma-runtime.js'
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
  expansionKind?: 'node' | 'cluster'
  expandNodeId?: string
  offset?: number
  hiddenCount?: number
}

interface GraphLink {
  source: string | NodeObject
  target: string | NodeObject
  label: string
}

interface GraphData {
  nodes: GraphNode[]
  links: GraphLink[]
}

interface NormalizedGraphLink {
  source: string
  target: string
  label: string
}

interface NormalizedGraphData {
  nodes: GraphNode[]
  links: NormalizedGraphLink[]
}

interface ForceGraphLink {
  label: string
}

interface SigmaNodeAttributes extends Record<string, unknown> {
  x: number
  y: number
  size: number
  color: string
  label: string
  hoverLabel: string
  isNewlyExpanded: boolean
  nodeType: string
}

interface SigmaEdgeAttributes extends Record<string, unknown> {
  size: number
  color: string
  label: string
  forceLabel: boolean
}

interface SigmaGraphViewProps {
  graphData: NormalizedGraphData
  labelNodeIds: Set<string>
  newlyExpandedNodeIds: Set<string>
  darkMode: boolean
  getNodeColor: (label: string) => string
  getEdgeColor: (label: string) => string
  onNodeClick: (nodeId: string) => void
}

interface SigmaLabelData {
  x: number
  y: number
  size: number
  label?: string
  hoverLabel?: string
  color: string
  isNewlyExpanded?: boolean
}

interface SigmaEdgeLabelData {
  key?: string
  label?: string
  size: number
  forceLabel?: boolean
}

interface SigmaEdgeLabelNodeData {
  x: number
  y: number
  size: number
}

class SigmaGraph<
  N extends Record<string, unknown> = Record<string, unknown>,
  E extends Record<string, unknown> = Record<string, unknown>,
> {
  private nodeAttributes = new Map<string, N>()
  private edgeRecords = new Map<string, { source: string; target: string; attributes: E }>()

  get order() {
    return this.nodeAttributes.size
  }

  addNode(key: string, attributes = {} as N): void {
    if (this.nodeAttributes.has(key)) throw new Error(`SigmaGraph: node "${key}" already exists.`)
    this.nodeAttributes.set(key, attributes)
  }

  addEdge(key: string, source: string, target: string, attributes = {} as E): void {
    if (this.edgeRecords.has(key)) throw new Error(`SigmaGraph: edge "${key}" already exists.`)
    if (!this.nodeAttributes.has(source)) this.addNode(source)
    if (!this.nodeAttributes.has(target)) this.addNode(target)
    this.edgeRecords.set(key, { source, target, attributes })
  }

  hasNode(key: string): boolean {
    return this.nodeAttributes.has(key)
  }

  hasEdge(key: string): boolean {
    return this.edgeRecords.has(key)
  }

  nodes(): string[] {
    return [...this.nodeAttributes.keys()]
  }

  edges(): string[] {
    return [...this.edgeRecords.keys()]
  }

  forEachNode(callback: (key: string, attributes: N) => void): void {
    this.nodeAttributes.forEach((attributes, key) => callback(key, attributes))
  }

  forEachEdge(callback: (key: string, attributes: E) => void): void {
    this.edgeRecords.forEach(({ attributes }, key) => callback(key, attributes))
  }

  getNodeAttributes(key: string): N {
    const attributes = this.nodeAttributes.get(key)
    if (!attributes) throw new Error(`SigmaGraph: node "${key}" not found.`)
    return attributes
  }

  getEdgeAttributes(key: string): E {
    const record = this.edgeRecords.get(key)
    if (!record) throw new Error(`SigmaGraph: edge "${key}" not found.`)
    return record.attributes
  }

  extremities(key: string): [string, string] {
    const record = this.edgeRecords.get(key)
    if (!record) throw new Error(`SigmaGraph: edge "${key}" not found.`)
    return [record.source, record.target]
  }

  on(): void {}

  removeListener(): void {}
}

function getEndpointId(endpoint: string | NodeObject): string {
  return typeof endpoint === 'object' ? String(endpoint.id) : endpoint
}

function normalizeGraphData(graphData: GraphData): NormalizedGraphData {
  return {
    nodes: graphData.nodes.map(node => ({ ...node })),
    links: graphData.links.map(link => ({
      source: getEndpointId(link.source),
      target: getEndpointId(link.target),
      label: link.label,
    })),
  }
}

const EXPANDER_PREFIX = '__expand__:'

function isExpanderNode(node: GraphNode) {
  return Boolean(node.expansionKind) || node.id.startsWith(EXPANDER_PREFIX)
}

function mergeGraphData(current: GraphData, incoming: GraphData, expandedNodeId?: string): GraphData {
  const nodesById = new Map<string, GraphNode>()
  current.nodes
    .filter(node => node.id !== expandedNodeId)
    .forEach(node => nodesById.set(node.id, { ...node }))
  incoming.nodes.forEach(node => nodesById.set(node.id, { ...node }))

  const linksByKey = new Map<string, GraphLink>()
  current.links
    .filter(link => getEndpointId(link.source) !== expandedNodeId && getEndpointId(link.target) !== expandedNodeId)
    .forEach(link => linksByKey.set(`${getEndpointId(link.source)}\t${getEndpointId(link.target)}\t${link.label}`, { ...link }))
  incoming.links.forEach(link => {
    linksByKey.set(`${getEndpointId(link.source)}\t${getEndpointId(link.target)}\t${link.label}`, { ...link })
  })

  return {
    nodes: [...nodesById.values()],
    links: [...linksByKey.values()],
  }
}

function realNodeIds(nodes: GraphNode[]): Set<string> {
  return new Set(nodes.filter(node => !isExpanderNode(node)).map(node => node.id))
}

function drawRoundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  context.beginPath()
  context.moveTo(x + radius, y)
  context.lineTo(x + width - radius, y)
  context.quadraticCurveTo(x + width, y, x + width, y + radius)
  context.lineTo(x + width, y + height - radius)
  context.quadraticCurveTo(x + width, y + height, x + width - radius, y + height)
  context.lineTo(x + radius, y + height)
  context.quadraticCurveTo(x, y + height, x, y + height - radius)
  context.lineTo(x, y + radius)
  context.quadraticCurveTo(x, y, x + radius, y)
  context.closePath()
}

function drawSigmaLabel(
  context: CanvasRenderingContext2D,
  data: SigmaLabelData,
  textColor: string,
  backgroundColor: string,
  strongBackground: boolean,
) {
  if (!data.label) return

  const fontSize = 13
  const paddingX = strongBackground ? 7 : 4
  const paddingY = strongBackground ? 4 : 2

  context.font = `600 ${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`
  context.textBaseline = 'middle'

  const textWidth = context.measureText(data.label).width
  const boxWidth = textWidth + paddingX * 2
  const boxHeight = fontSize + paddingY * 2
  const canvasWidth = context.canvas.width
  const canvasHeight = context.canvas.height
  const margin = 6
  const preferredX = data.x + data.size + 5 - paddingX
  const preferredY = data.y - fontSize / 2 - paddingY
  const boxX = Math.min(Math.max(preferredX, margin), Math.max(margin, canvasWidth - boxWidth - margin))
  const boxY = Math.min(Math.max(preferredY, margin), Math.max(margin, canvasHeight - boxHeight - margin))
  const textX = boxX + paddingX
  const textY = boxY + boxHeight / 2

  context.save()
  context.fillStyle = backgroundColor
  drawRoundedRect(context, boxX, boxY, boxWidth, boxHeight, strongBackground ? 6 : 4)
  context.fill()

  context.fillStyle = textColor
  context.shadowColor = strongBackground ? 'transparent' : backgroundColor
  context.shadowBlur = strongBackground ? 0 : 3
  context.fillText(data.label, textX, textY)
  context.restore()
}

function drawSigmaNode(
  context: CanvasRenderingContext2D,
  data: SigmaLabelData,
  highlighted: boolean,
) {
  context.save()
  if (highlighted) {
    context.fillStyle = 'rgba(245, 158, 11, 0.22)'
    context.beginPath()
    context.arc(data.x, data.y, data.size + 7, 0, Math.PI * 2)
    context.fill()
  }

  context.fillStyle = data.color
  context.beginPath()
  context.arc(data.x, data.y, data.size, 0, Math.PI * 2)
  context.fill()

  if (highlighted) {
    context.strokeStyle = '#f59e0b'
    context.lineWidth = 3
    context.beginPath()
    context.arc(data.x, data.y, data.size + 2, 0, Math.PI * 2)
    context.stroke()
  }
  context.restore()
}

function drawSigmaEdgeLabel(
  context: CanvasRenderingContext2D,
  edgeData: SigmaEdgeLabelData,
  sourceData: SigmaEdgeLabelNodeData,
  targetData: SigmaEdgeLabelNodeData,
  hoveredEdgeId: string | null,
  textColor: string,
  backgroundColor: string,
) {
  if (edgeData.key !== hoveredEdgeId) return
  if (!edgeData.label) return

  const dx = targetData.x - sourceData.x
  const dy = targetData.y - sourceData.y
  const distance = Math.hypot(dx, dy)
  if (distance < sourceData.size + targetData.size + 18) return

  const fontSize = 11
  const paddingX = 5
  const paddingY = 3
  const unitX = dx / distance
  const unitY = dy / distance
  const startX = sourceData.x + unitX * sourceData.size
  const startY = sourceData.y + unitY * sourceData.size
  const endX = targetData.x - unitX * targetData.size
  const endY = targetData.y - unitY * targetData.size
  const midX = (startX + endX) / 2
  const midY = (startY + endY) / 2
  const availableWidth = Math.max(12, Math.hypot(endX - startX, endY - startY) - 8)

  context.save()
  context.font = `600 ${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`
  context.textBaseline = 'middle'

  let label = edgeData.label
  let textWidth = context.measureText(label).width
  if (textWidth > availableWidth) {
    while (label.length > 1 && context.measureText(`${label}...`).width > availableWidth) {
      label = label.slice(0, -1)
    }
    label = `${label}...`
    textWidth = context.measureText(label).width
  }

  const boxWidth = textWidth + paddingX * 2
  const boxHeight = fontSize + paddingY * 2
  const boxX = midX - boxWidth / 2
  const boxY = midY - boxHeight / 2

  context.fillStyle = backgroundColor
  drawRoundedRect(context, boxX, boxY, boxWidth, boxHeight, 4)
  context.fill()
  context.fillStyle = textColor
  context.fillText(label, boxX + paddingX, midY)
  context.restore()
}

function createInitialLayout(graphData: NormalizedGraphData) {
  const nodeCount = Math.max(1, graphData.nodes.length)
  const degrees: Record<string, number> = {}
  const positions: Record<string, { x: number; y: number }> = {}

  graphData.nodes.forEach(node => {
    degrees[node.id] = 0
  })

  graphData.links.forEach(link => {
    degrees[link.source] = (degrees[link.source] || 0) + 1
    degrees[link.target] = (degrees[link.target] || 0) + 1
  })

  const rankedNodes = [...graphData.nodes].sort((a, b) => (degrees[b.id] || 0) - (degrees[a.id] || 0))
  const radius = Math.max(4, Math.sqrt(nodeCount) * 2.4)

  rankedNodes.forEach((node, index) => {
    const angle = index * Math.PI * (3 - Math.sqrt(5))
    const ring = radius * Math.sqrt((index + 0.5) / nodeCount)
    positions[node.id] = {
      x: Math.cos(angle) * ring,
      y: Math.sin(angle) * ring,
    }
  })

  return { degrees, positions }
}

function SigmaGraphView({ graphData, labelNodeIds, newlyExpandedNodeIds, darkMode, getNodeColor, getEdgeColor, onNodeClick }: SigmaGraphViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const rendererRef = useRef<Sigma | null>(null)
  const hoveredEdgeRef = useRef<string | null>(null)

  const graph = useMemo(() => {
    const { degrees, positions } = createInitialLayout(graphData)
    const maxDegree = Math.max(1, ...Object.values(degrees))
    const sigmaGraph = new SigmaGraph<SigmaNodeAttributes, SigmaEdgeAttributes>()

    graphData.nodes.forEach(node => {
      const position = positions[node.id] || { x: 0, y: 0 }
      const degree = degrees[node.id] || 0
      sigmaGraph.addNode(node.id, {
        x: position.x,
        y: position.y,
        size: 4 + (degree / maxDegree) * 14,
        color: isExpanderNode(node) ? '#f59e0b' : getNodeColor(node.label),
        label: isExpanderNode(node) || labelNodeIds.has(node.id) ? node.name || node.id : '',
        hoverLabel: node.name || node.id,
        isNewlyExpanded: newlyExpandedNodeIds.has(node.id),
        nodeType: node.label,
      })
    })

    const edgeCounts = new Map<string, number>()
    graphData.links.forEach((link, index) => {
      if (!sigmaGraph.hasNode(link.source) || !sigmaGraph.hasNode(link.target)) return
      const pairKey = `${link.source}->${link.target}`
      const pairIndex = edgeCounts.get(pairKey) || 0
      edgeCounts.set(pairKey, pairIndex + 1)
      const edgeKey = `${pairKey}#${pairIndex}-${index}`
      const edgeLabel = link.label === 'more' ? '' : link.label || ''
      sigmaGraph.addEdge(edgeKey, link.source, link.target, {
        size: 1.8,
        color: getEdgeColor(link.label || 'edge'),
        label: edgeLabel,
        forceLabel: Boolean(edgeLabel),
      })
    })

    return sigmaGraph
  }, [graphData, labelNodeIds, newlyExpandedNodeIds, getNodeColor, getEdgeColor])

  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const labelTextColor = '#111827'
    const labelBackgroundColor = darkMode ? 'rgba(248, 250, 252, 0.94)' : 'rgba(255, 255, 255, 0.9)'
    const expandedTextColor = '#111827'
    const expandedBackgroundColor = 'rgba(254, 243, 199, 0.96)'
    const hoverTextColor = '#111827'
    const hoverBackgroundColor = darkMode ? 'rgba(255, 255, 255, 0.98)' : 'rgba(255, 255, 255, 0.98)'
    const edgeLabelTextColor = '#111827'
    const edgeLabelBackgroundColor = darkMode ? 'rgba(248, 250, 252, 0.92)' : 'rgba(255, 255, 255, 0.92)'

    rendererRef.current?.kill()
    rendererRef.current = new Sigma(graph, container, {
      allowInvalidContainer: true,
      defaultEdgeType: 'arrow',
      enableEdgeEvents: true,
      labelColor: { color: labelTextColor },
      renderEdgeLabels: true,
      edgeLabelColor: { color: edgeLabelTextColor },
      edgeLabelSize: 11,
      edgeLabelWeight: '600',
      labelRenderedSizeThreshold: 0,
      minCameraRatio: 0.03,
      maxCameraRatio: 12,
      defaultDrawEdgeLabel: (context, edgeData, sourceData, targetData) => {
        drawSigmaEdgeLabel(
          context,
          edgeData,
          sourceData,
          targetData,
          hoveredEdgeRef.current,
          edgeLabelTextColor,
          edgeLabelBackgroundColor,
        )
      },
      defaultDrawNodeLabel: (context, data) => {
        drawSigmaLabel(
          context,
          data,
          data.isNewlyExpanded ? expandedTextColor : labelTextColor,
          data.isNewlyExpanded ? expandedBackgroundColor : labelBackgroundColor,
          Boolean(data.isNewlyExpanded),
        )
      },
      defaultDrawNodeHover: (context, data) => {
        const labelData = {
          ...data,
          label: typeof data.hoverLabel === 'string' ? data.hoverLabel : data.label,
        }

        drawSigmaNode(context, data, Boolean(data.isNewlyExpanded))

        drawSigmaLabel(context, labelData, hoverTextColor, hoverBackgroundColor, true)
      },
    })
    rendererRef.current.on('clickNode', ({ node }: { node: string }) => {
      onNodeClick(node)
    })
    rendererRef.current.on('enterEdge', ({ edge }: { edge: string }) => {
      hoveredEdgeRef.current = edge
      rendererRef.current?.refresh()
    })
    rendererRef.current.on('leaveEdge', () => {
      hoveredEdgeRef.current = null
      rendererRef.current?.refresh()
    })

    rendererRef.current.refresh()

    return () => {
      rendererRef.current?.kill()
      rendererRef.current = null
    }
  }, [graph, darkMode, onNodeClick])

  return <div ref={containerRef} className="sigma-canvas" />
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
  const [renderer, setRenderer] = useState<'sigma' | 'force'>('sigma')
  const [lastExpandedNodeIds, setLastExpandedNodeIds] = useState<Set<string>>(() => new Set())
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const graphRef = useRef<any>(null)
  const graphContainerRef = useRef<HTMLDivElement | null>(null)
  const [graphSize, setGraphSize] = useState({ width: 1, height: 1 })
  const customQueryRef = useRef<string>('')
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const fetchDatabases = () => {
    Promise.all([
      invoke<Database[]>('get_databases'),
      invoke<number | null>('get_initial_database_id'),
    ])
      .then(([items, initialId]) => {
        setDatabases(items)
        if (typeof initialId === 'number') {
          setSelectedId(initialId)
        }
      })
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

  useEffect(() => {
    const container = graphContainerRef.current
    if (!container) return

    const updateGraphSize = () => {
      const rect = container.getBoundingClientRect()
      const width = Math.max(1, Math.floor(rect.width))
      const height = Math.max(1, Math.floor(rect.height))
      setGraphSize(current => (
        current.width === width && current.height === height
          ? current
          : { width, height }
      ))
    }

    updateGraphSize()
    const observer = new ResizeObserver(updateGraphSize)
    observer.observe(container)
    window.addEventListener('resize', updateGraphSize)

    return () => {
      observer.disconnect()
      window.removeEventListener('resize', updateGraphSize)
    }
  }, [])

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
          setLastExpandedNodeIds(new Set())
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
          setLastExpandedNodeIds(new Set())
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

  const handleNodeClick = useCallback((nodeId: string) => {
    const node = graphData.nodes.find(item => item.id === nodeId)
    if (!node?.expansionKind || node.expansionKind !== 'node' || !node.expandNodeId) return

    setLoading(true)
    setError(null)
    invoke<GraphData>('expand_node', {
      id: selectedId,
      nodeId: node.expandNodeId,
      visibleNodeIds: graphData.nodes.map(item => item.id),
      offset: node.offset ?? 0,
    })
      .then(data => {
        const beforeNodeIds = realNodeIds(graphData.nodes)
        const returnedNodeIds = realNodeIds(data.nodes)
        const merged = mergeGraphData(graphData, data, node.id)
        const highlightedNodeIds = realNodeIds(merged.nodes)

        beforeNodeIds.forEach(id => {
          if (!returnedNodeIds.has(id)) highlightedNodeIds.delete(id)
        })
        setGraphData(merged)
        setLastExpandedNodeIds(highlightedNodeIds)
        setLoading(false)
      })
      .catch(err => {
        setError(String(err))
        setLoading(false)
      })
  }, [graphData, selectedId])

  const normalizedGraphData = useMemo(() => normalizeGraphData(graphData), [graphData])
  const forceGraphData = useMemo<ForceGraphData<GraphNode, ForceGraphLink>>(() => ({
    nodes: normalizedGraphData.nodes.map(node => ({ ...node })),
    links: normalizedGraphData.links.map(link => ({ ...link })),
  }), [normalizedGraphData])

  const nodeDegree = useMemo(() => {
    const degrees: Record<string, number> = {}
    normalizedGraphData.nodes.forEach(n => degrees[n.id] = 0)
    normalizedGraphData.links.forEach(link => {
      degrees[link.source] = (degrees[link.source] || 0) + 1
      degrees[link.target] = (degrees[link.target] || 0) + 1
    })
    return degrees
  }, [normalizedGraphData])

  const maxDegree = useMemo(() => Math.max(1, ...Object.values(nodeDegree)), [nodeDegree])

  const topLabelNodeIds = useMemo(() => {
    return new Set(
      [
        ...lastExpandedNodeIds,
        ...[...normalizedGraphData.nodes]
        .sort((a, b) => (nodeDegree[b.id] || 0) - (nodeDegree[a.id] || 0))
        .filter(node => !isExpanderNode(node))
        .slice(0, 5)
        .map(node => node.id),
      ]
    )
  }, [lastExpandedNodeIds, normalizedGraphData.nodes, nodeDegree])

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

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const paintNode = useCallback((node: any, ctx: CanvasRenderingContext2D) => {
    const size = getNodeSize(node)
    const color = isExpanderNode(node) ? '#f59e0b' : getNodeColor(node.label)
    const highlighted = lastExpandedNodeIds.has(node.id)

    if (highlighted) {
      ctx.fillStyle = 'rgba(245, 158, 11, 0.22)'
      ctx.beginPath()
      ctx.arc(node.x, node.y, size + 7, 0, 2 * Math.PI)
      ctx.fill()
    }

    ctx.fillStyle = color
    ctx.beginPath()
    ctx.arc(node.x, node.y, size, 0, 2 * Math.PI)
    ctx.fill()

    ctx.strokeStyle = highlighted ? '#f59e0b' : darkMode ? '#222' : '#ddd'
    ctx.lineWidth = highlighted ? 3 : 1
    ctx.stroke()

    if ((isExpanderNode(node) || topLabelNodeIds.has(node.id)) && node.name) {
      const fontSize = 3
      ctx.font = `${fontSize}px Sans-Serif`
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillStyle = highlighted ? '#f59e0b' : '#fff'

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
  }, [getNodeSize, getNodeColor, darkMode, topLabelNodeIds, lastExpandedNodeIds])

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
                  onClick={() => {
                    setSelectedId(db.id)
                  }}
                  title={db.relativePath}
                >
                  {db.name}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        <div className="header">
          <div className="header-left">
            <span className="graph-stats">
              {loading ? 'Loading...' : `${graphData.nodes.length} nodes, ${graphData.links.length} edges`}
            </span>
            {error && <span className="error-message">{error}</span>}
          </div>

          <div className="header-right">
            <div className="renderer-toggle" aria-label="Renderer">
              <button
                className={renderer === 'sigma' ? 'active' : ''}
                onClick={() => setRenderer('sigma')}
              >
                Sigma
              </button>
              <button
                className={renderer === 'force' ? 'active' : ''}
                onClick={() => setRenderer('force')}
              >
                Force
              </button>
            </div>
            <button
              className="theme-toggle"
              onClick={() => setDarkMode(!darkMode)}
            >
              {darkMode ? '☀️ Light' : '🌙 Dark'}
            </button>
          </div>
        </div>

        <div className="graph-container" ref={graphContainerRef}>
          {!loading && !error && graphData.nodes.length > 0 && renderer === 'sigma' && (
            <SigmaGraphView
              key="sigma"
              graphData={normalizedGraphData}
              labelNodeIds={topLabelNodeIds}
              newlyExpandedNodeIds={lastExpandedNodeIds}
              darkMode={darkMode}
              getNodeColor={getNodeColor}
              getEdgeColor={getEdgeColor}
              onNodeClick={handleNodeClick}
            />
          )}
          {!loading && !error && graphData.nodes.length > 0 && renderer === 'force' && (
            <ForceGraph2D
              key="force"
              ref={graphRef}
              width={graphSize.width}
              height={graphSize.height}
              graphData={forceGraphData}
              nodeCanvasObject={paintNode}
              onNodeClick={(node) => handleNodeClick(String(node.id))}
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
