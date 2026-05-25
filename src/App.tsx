import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import ForceGraph2D from 'react-force-graph-2d'
import type { GraphData as ForceGraphData, NodeObject } from 'react-force-graph-2d'
import { IcebugSigmaGraph, Sigma } from './vendor/sigma-runtime.js'
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
  tableId?: number
  rowid?: number
  community?: number
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

interface GraphCsr {
  indptr: number[]
  indices: number[]
  edgeIds?: number[] | null
}

interface GraphCluster {
  clusterId: number
  label: string
  size: number
  parentClusterId?: number | null
}

interface GraphClusterLevel {
  level: number
  membership: number[]
  clusters: GraphCluster[]
}

interface GraphClusterDebug {
  enabled: boolean
  status: string
  message: string
  nodeCount: number
  edgeCount: number
  undirectedEdgeCount: number
  levels: number
  clusters: number
}

interface GraphData {
  nodes: GraphNode[]
  links: GraphLink[]
  csr?: GraphCsr
  clusterLevels?: GraphClusterLevel[]
  clusterDebug?: GraphClusterDebug
}

interface NormalizedGraphLink {
  source: string
  target: string
  label: string
}

interface NormalizedGraphData {
  nodes: GraphNode[]
  links: NormalizedGraphLink[]
  csr?: GraphCsr
  clusterLevels?: GraphClusterLevel[]
  clusterDebug?: GraphClusterDebug
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
  getNodeColor: (node: GraphNode) => string
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
    csr: graphData.csr,
    clusterLevels: graphData.clusterLevels,
    clusterDebug: graphData.clusterDebug,
  }
}

const EXPANDER_PREFIX = '__expand__:'

function isExpanderNode(node: GraphNode) {
  return Boolean(node.expansionKind) || node.id.startsWith(EXPANDER_PREFIX)
}

function buildCommunityClusterLevels(graphData: NormalizedGraphData): GraphClusterLevel[] {
  if (graphData.clusterLevels?.length) return graphData.clusterLevels

  const communityIds = graphData.nodes.map(node => node.community)
  if (communityIds.some(id => id === undefined)) return []

  const counts = new Map<number, number>()
  communityIds.forEach(id => {
    if (id !== undefined) counts.set(id, (counts.get(id) || 0) + 1)
  })

  return [{
    level: 0,
    membership: communityIds.map(id => id ?? 0),
    clusters: [...counts.entries()].map(([clusterId, size]) => ({
      clusterId,
      label: `Community ${clusterId}`,
      size,
    })),
  }]
}

function getClusterNodeId(level: number, clusterId: number) {
  return `__cluster__:${level}:${clusterId}`
}

function parseClusterNodeId(nodeId: string): { level: number; clusterId: number } | null {
  const match = /^__cluster__:(\d+):(\d+)$/.exec(nodeId)
  if (!match) return null
  return {
    level: Number(match[1]),
    clusterId: Number(match[2]),
  }
}

function collapseGraphByClusterLevel(
  graphData: NormalizedGraphData,
  level: GraphClusterLevel,
  expandedClusterId: number | null = null,
): NormalizedGraphData {
  const clusterById = new Map(level.clusters.map(cluster => [cluster.clusterId, cluster]))
  const clusterCounts = new Map<number, number>()
  const nodeIndex = new Map(graphData.nodes.map((node, index) => [node.id, index]))

  level.membership.forEach(clusterId => {
    clusterCounts.set(clusterId, (clusterCounts.get(clusterId) || 0) + 1)
  })

  const collapsedNodes: GraphNode[] = [...clusterCounts.entries()]
    .filter(([clusterId]) => clusterId !== expandedClusterId)
    .sort(([a], [b]) => a - b)
    .map(([clusterId, size]) => {
      const cluster = clusterById.get(clusterId)
      return {
        id: getClusterNodeId(level.level, clusterId),
        name: cluster?.label || `Cluster ${clusterId}`,
        label: `Cluster L${level.level}`,
        community: clusterId,
        expansionKind: 'cluster',
        hiddenCount: cluster?.size ?? size,
      }
    })

  const expandedNodes = expandedClusterId === null
    ? []
    : graphData.nodes.filter((_, index) => level.membership[index] === expandedClusterId)
  const nodes = [...collapsedNodes, ...expandedNodes]
  const visibleNodeIds = new Set(nodes.map(node => node.id))
  const edgeCounts = new Map<string, { source: string; target: string; labels: Map<string, number>; count: number }>()

  graphData.links.forEach(link => {
    const sourceIndex = nodeIndex.get(link.source)
    const targetIndex = nodeIndex.get(link.target)
    if (sourceIndex === undefined || targetIndex === undefined) return

    const sourceCluster = level.membership[sourceIndex]
    const targetCluster = level.membership[targetIndex]

    const source = sourceCluster === expandedClusterId
      ? link.source
      : getClusterNodeId(level.level, sourceCluster)
    const target = targetCluster === expandedClusterId
      ? link.target
      : getClusterNodeId(level.level, targetCluster)
    if (source === target || !visibleNodeIds.has(source) || !visibleNodeIds.has(target)) return

    const key = `${source}\t${target}`
    const record = edgeCounts.get(key) || { source, target, labels: new Map<string, number>(), count: 0 }
    record.count += 1
    record.labels.set(link.label, (record.labels.get(link.label) || 0) + 1)
    edgeCounts.set(key, record)
  })

  const links = [...edgeCounts.values()].map(record => {
    const label = record.count === 1
      ? [...record.labels.keys()][0] || 'edge'
      : `${record.count} edges`
    return { source: record.source, target: record.target, label }
  })

  return {
    nodes,
    links,
    csr: buildGraphCsr({ nodes, links }),
    clusterLevels: graphData.clusterLevels,
    clusterDebug: graphData.clusterDebug,
  }
}

function buildGraphCsr(graphData: NormalizedGraphData): GraphCsr {
  if (
    graphData.csr &&
    graphData.csr.indptr.length === graphData.nodes.length + 1 &&
    graphData.csr.indices.length === graphData.links.length
  ) {
    return graphData.csr
  }

  const nodeIndex = new Map(graphData.nodes.map((node, index) => [node.id, index]))
  const outgoing: Array<Array<{ target: number; edgeIndex: number }>> = Array.from({ length: graphData.nodes.length }, () => [])

  graphData.links.forEach((link, edgeIndex) => {
    const sourceIndex = nodeIndex.get(link.source)
    const targetIndex = nodeIndex.get(link.target)
    if (sourceIndex === undefined || targetIndex === undefined) return
    outgoing[sourceIndex].push({ target: targetIndex, edgeIndex })
  })

  const indptr: number[] = []
  const indices: number[] = []
  const edgeIds: number[] = []

  outgoing.forEach(neighbors => {
    indptr.push(indices.length)
    neighbors.forEach(({ target, edgeIndex }) => {
      indices.push(target)
      edgeIds.push(edgeIndex)
    })
  })
  indptr.push(indices.length)

  return { indptr, indices, edgeIds }
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
    const nodes = graphData.nodes.map(node => {
      const position = positions[node.id] || { x: 0, y: 0 }
      const degree = degrees[node.id] || 0
      return {
        key: node.id,
        attributes: {
          x: position.x,
          y: position.y,
          size: 4 + (degree / maxDegree) * 14,
          color: isExpanderNode(node) ? '#f59e0b' : getNodeColor(node),
          label: isExpanderNode(node) || labelNodeIds.has(node.id) ? node.name || node.id : '',
          hoverLabel: node.name || node.id,
          isNewlyExpanded: newlyExpandedNodeIds.has(node.id),
          nodeType: node.label,
        },
      }
    })

    const edgeCounts = new Map<string, number>()
    const edgeAttributes: SigmaEdgeAttributes[] = graphData.links.map(link => {
      const edgeLabel = link.label === 'more' ? '' : link.label || ''
      return {
        size: 1.8,
        color: getEdgeColor(link.label || 'edge'),
        label: edgeLabel,
        forceLabel: Boolean(edgeLabel),
      }
    })
    const edgeKeys: string[] = graphData.links.map((link, index) => {
      const pairKey = `${link.source}->${link.target}`
      const pairIndex = edgeCounts.get(pairKey) || 0
      edgeCounts.set(pairKey, pairIndex + 1)
      return `${pairKey}#${pairIndex}-${index}`
    })
    const csr = buildGraphCsr(graphData)

    return new IcebugSigmaGraph<SigmaNodeAttributes, SigmaEdgeAttributes>({
      directed: true,
      nodes,
      csr: {
        indptr: new BigUint64Array(csr.indptr.map(BigInt)),
        indices: new BigUint64Array(csr.indices.map(BigInt)),
        edgeIds: csr.edgeIds ? new BigUint64Array(csr.edgeIds.map(BigInt)) : null,
      },
      edgeAttributes,
      edgeKeys,
    })
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
  const [clusterCollapsed, setClusterCollapsed] = useState(false)
  const [selectedClusterLevel, setSelectedClusterLevel] = useState(0)
  const [expandedClusterId, setExpandedClusterId] = useState<number | null>(null)
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
          console.info('Graph cluster debug:', data.clusterDebug)
          setGraphData(data)
          setLastExpandedNodeIds(new Set())
          setExpandedClusterId(null)
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
          console.info('Graph cluster debug:', data.clusterDebug)
          setGraphData(data)
          setLastExpandedNodeIds(new Set())
          setExpandedClusterId(null)
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
        const highlightedNodeIds = realNodeIds(data.nodes)

        beforeNodeIds.forEach(id => {
          highlightedNodeIds.delete(id)
        })
        setGraphData(data)
        setLastExpandedNodeIds(highlightedNodeIds)
        setExpandedClusterId(null)
        setLoading(false)
      })
      .catch(err => {
        setError(String(err))
        setLoading(false)
      })
  }, [graphData, selectedId])

  const normalizedGraphData = useMemo(() => normalizeGraphData(graphData), [graphData])
  const clusterLevels = useMemo(() => buildCommunityClusterLevels(normalizedGraphData), [normalizedGraphData])
  const selectedCluster = useMemo(() => (
    clusterLevels.find(level => level.level === selectedClusterLevel) || clusterLevels[0]
  ), [clusterLevels, selectedClusterLevel])
  const expandedCluster = useMemo(() => (
    expandedClusterId === null
      ? null
      : selectedCluster?.clusters.find(cluster => cluster.clusterId === expandedClusterId) || null
  ), [expandedClusterId, selectedCluster])
  const visibleGraphData = useMemo(() => (
    clusterCollapsed && selectedCluster
      ? collapseGraphByClusterLevel(normalizedGraphData, selectedCluster, expandedClusterId)
      : normalizedGraphData
  ), [clusterCollapsed, expandedClusterId, normalizedGraphData, selectedCluster])

  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    if (!selectedCluster) {
      setClusterCollapsed(false)
      setSelectedClusterLevel(0)
      setExpandedClusterId(null)
      return
    }
    if (!clusterLevels.some(level => level.level === selectedClusterLevel)) {
      setSelectedClusterLevel(selectedCluster.level)
      setExpandedClusterId(null)
    }
  }, [clusterLevels, selectedCluster, selectedClusterLevel])
  /* eslint-enable react-hooks/set-state-in-effect */

  const forceGraphData = useMemo<ForceGraphData<GraphNode, ForceGraphLink>>(() => ({
    nodes: visibleGraphData.nodes.map(node => ({ ...node })),
    links: visibleGraphData.links.map(link => ({ ...link })),
  }), [visibleGraphData])

  const nodeDegree = useMemo(() => {
    const degrees: Record<string, number> = {}
    visibleGraphData.nodes.forEach(n => degrees[n.id] = 0)
    visibleGraphData.links.forEach(link => {
      degrees[link.source] = (degrees[link.source] || 0) + 1
      degrees[link.target] = (degrees[link.target] || 0) + 1
    })
    return degrees
  }, [visibleGraphData])

  const maxDegree = useMemo(() => Math.max(1, ...Object.values(nodeDegree)), [nodeDegree])

  const topLabelNodeIds = useMemo(() => {
    return new Set(
      [
        ...lastExpandedNodeIds,
        ...[...visibleGraphData.nodes]
        .sort((a, b) => (nodeDegree[b.id] || 0) - (nodeDegree[a.id] || 0))
        .filter(node => !isExpanderNode(node))
        .slice(0, 5)
        .map(node => node.id),
      ]
    )
  }, [lastExpandedNodeIds, visibleGraphData.nodes, nodeDegree])

  const getNodeColor = useCallback((node: GraphNode) => {
    const key = node.community === undefined ? node.label : `community:${node.community}`
    if (!colorMapRef.current[key]) {
      const colors = ['#4e79a7', '#f28e2c', '#e15759', '#76b7b2', '#59a14f', '#edc949', '#af7aa1', '#ff9da7', '#9c755f', '#bab0ab']
      colorMapRef.current[key] = colors[Object.keys(colorMapRef.current).length % colors.length]
    }
    return colorMapRef.current[key]
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
    const clusterBoost = node.expansionKind === 'cluster' ? Math.min(10, Math.sqrt(node.hiddenCount || 1)) : 0
    return 4 + clusterBoost + (degree / maxDegree) * 12
  }, [nodeDegree, maxDegree])

  const handleVisibleNodeClick = useCallback((nodeId: string) => {
    const clusterNode = parseClusterNodeId(nodeId)
    if (clusterNode) {
      setSelectedClusterLevel(clusterNode.level)
      setExpandedClusterId(clusterNode.clusterId)
      setClusterCollapsed(true)
      return
    }
    handleNodeClick(nodeId)
  }, [handleNodeClick])

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const paintNode = useCallback((node: any, ctx: CanvasRenderingContext2D) => {
    const size = getNodeSize(node)
    const color = isExpanderNode(node) ? '#f59e0b' : getNodeColor(node)
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
              {loading
                ? 'Loading...'
                : clusterCollapsed && selectedCluster
                  ? expandedCluster
                    ? `${expandedCluster.label} expanded, ${visibleGraphData.nodes.length} visible nodes, ${visibleGraphData.links.length} visible edges`
                    : `${visibleGraphData.nodes.length} clusters, ${visibleGraphData.links.length} aggregate edges`
                  : `${graphData.nodes.length} nodes, ${graphData.links.length} edges`}
            </span>
            {error && <span className="error-message">{error}</span>}
          </div>

          <div className="header-right">
            {clusterLevels.length > 0 && (
              <div className="cluster-controls" aria-label="Clusters">
                <select
                  value={selectedCluster?.level ?? 0}
                  onChange={event => {
                    setSelectedClusterLevel(Number(event.target.value))
                    setExpandedClusterId(null)
                  }}
                >
                  {clusterLevels.map(level => (
                    <option key={level.level} value={level.level}>
                      Level {level.level}
                    </option>
                  ))}
                </select>
                <button
                  className={clusterCollapsed ? 'active' : ''}
                  onClick={() => {
                    setExpandedClusterId(null)
                    setClusterCollapsed(value => !value)
                  }}
                >
                  {clusterCollapsed ? 'Expand All' : 'Collapse'}
                </button>
              </div>
            )}
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
              graphData={visibleGraphData}
              labelNodeIds={topLabelNodeIds}
              newlyExpandedNodeIds={lastExpandedNodeIds}
              darkMode={darkMode}
              getNodeColor={getNodeColor}
              getEdgeColor={getEdgeColor}
              onNodeClick={handleVisibleNodeClick}
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
              onNodeClick={(node) => handleVisibleNodeClick(String(node.id))}
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
