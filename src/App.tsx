import { useState, useEffect, useMemo, useCallback, useRef } from 'react'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { tableFromIPC } from 'apache-arrow'
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
  properties?: Record<string, string>
  tableId?: number
  rowid?: number
  community?: number
  expansionKind?: 'node' | 'cluster'
  expandNodeId?: string
  offset?: number
  hiddenCount?: number
  colorKey?: string
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

interface ClusterPathItem {
  level: number
  clusterId: number
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
  csrArrowIpc?: number[] | Uint8Array
  clusterLevels?: GraphClusterLevel[]
  clusterDebug?: GraphClusterDebug
}

interface LlmClusterNamingConfig {
  accessToken: string
  endpoint?: string
  model?: string
  sampleSize: number
}

interface LlmClusterNameRequest {
  key: string
  clusterId: number
  labels: string[]
}

interface LlmClusterNameResult {
  key: string
  name?: string | null
  error?: string | null
}

type GraphViewMode = 'data' | 'schema'

interface NormalizedGraphLink {
  source: string
  target: string
  label: string
}

interface NormalizedGraphData {
  nodes: GraphNode[]
  links: NormalizedGraphLink[]
  csr?: GraphCsr
  csrArrowIpc?: number[] | Uint8Array
  clusterLevels?: GraphClusterLevel[]
  clusterDebug?: GraphClusterDebug
}

interface GraphCsrArrays {
  indptr: BigUint64Array
  indices: BigUint64Array
  edgeIds: BigUint64Array | null
}

interface ForceGraphLink {
  label: string
}

type ForceGraphNodeObject = GraphNode & {
  x?: number
  y?: number
}

type ForceGraphPosition = ForceGraphNodeObject & {
  x: number
  y: number
}

type ForceGraphLinkObject = ForceGraphLink & {
  source?: string | number | ForceGraphNodeObject
  target?: string | number | ForceGraphNodeObject
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
  alwaysShowEdgeLabels: boolean
  getNodeDisplayName: (node: GraphNode) => string
  getNodeColor: (node: GraphNode) => string
  getNodeSize: (node: GraphNode) => number
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

function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return Promise.reject(new Error('Native app bridge is unavailable. Open this screen through the Tauri desktop app.'))
  }
  return invoke<T>(cmd, args)
}

function buildLlmClusterConfig(config: LlmClusterNamingConfig): LlmClusterNamingConfig | null {
  const accessToken = config.accessToken.trim()
  if (!accessToken) return null
  const endpoint = config.endpoint?.trim()
  const model = config.model?.trim()
  return {
    accessToken,
    sampleSize: config.sampleSize,
    ...(endpoint ? { endpoint } : {}),
    ...(model ? { model } : {}),
  }
}

const AUTO_DISPLAY_COLUMN = '__auto__'

function getNodeDisplayName(node: GraphNode, displayColumns: Record<string, string>) {
  if (isExpanderNode(node) || isClusterNode(node)) return node.name || node.id

  const selectedColumn = displayColumns[node.label]
  if (selectedColumn && selectedColumn !== AUTO_DISPLAY_COLUMN) {
    const selectedValue = node.properties?.[selectedColumn]
    if (selectedValue?.trim()) return selectedValue
  }

  return node.name || node.id
}

function getNodeClusterLabel(node: GraphNode, displayColumns: Record<string, string>) {
  const label = node.label.trim()
  const name = getNodeDisplayName(node, displayColumns).trim()
  return !name || name === label ? label : `${label}: ${name}`
}

function sampleLabels(labels: string[], sampleSize: number) {
  const unique = [...new Set(labels)].filter(Boolean)
  for (let index = unique.length - 1; index > 0; index -= 1) {
    const swapIndex = Math.floor(Math.random() * (index + 1))
    const value = unique[index]
    unique[index] = unique[swapIndex]
    unique[swapIndex] = value
  }
  return unique.slice(0, sampleSize)
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
    csrArrowIpc: graphData.csrArrowIpc,
    clusterLevels: graphData.clusterLevels,
    clusterDebug: graphData.clusterDebug,
  }
}

function expandSchemaSelfLinks(graphData: NormalizedGraphData): NormalizedGraphData {
  const nodes = graphData.nodes.map(node => ({ ...node }))
  const nodeById = new Map(nodes.map(node => [node.id, node]))
  const links: NormalizedGraphLink[] = []

  graphData.links.forEach((link, index) => {
    if (link.source !== link.target) {
      links.push({ ...link })
      return
    }

    const sourceNode = nodeById.get(link.source)
    const duplicateId = `__schema_self_target__:${link.label}:${link.target}:${index}`
    nodes.push({
      ...(sourceNode || {
        id: link.target,
        name: link.target,
        label: 'Node Table',
      }),
      id: duplicateId,
      properties: {},
    })
    links.push({
      ...link,
      target: duplicateId,
    })
  })

  return {
    ...graphData,
    nodes,
    links,
    csr: buildGraphCsr({ ...graphData, nodes, links }),
    csrArrowIpc: undefined,
  }
}

const EXPANDER_PREFIX = '__expand__:'

function isExpanderNode(node: GraphNode) {
  return node.expansionKind === 'node' || node.id.startsWith(EXPANDER_PREFIX)
}

function isClusterNode(node: GraphNode) {
  return node.expansionKind === 'cluster'
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

function getCoarsestClusterLevel(clusterLevels: GraphClusterLevel[]) {
  return clusterLevels.reduce<GraphClusterLevel | null>((coarsest, level) => (
    !coarsest || level.level > coarsest.level ? level : coarsest
  ), null)
}

function getClusterLevel(clusterLevels: GraphClusterLevel[], level: number) {
  return clusterLevels.find(item => item.level === level) || null
}

function nodeMatchesClusterPath(
  clusterLevels: GraphClusterLevel[],
  nodeIndex: number,
  clusterPath: ClusterPathItem[],
) {
  return clusterPath.every(pathItem => (
    getClusterLevel(clusterLevels, pathItem.level)?.membership[nodeIndex] === pathItem.clusterId
  ))
}

function clusterBelongsToPath(cluster: GraphCluster, clusterPath: ClusterPathItem[]) {
  const parent = clusterPath[clusterPath.length - 1]
  return !parent || cluster.parentClusterId === parent.clusterId
}

function getClusterPathForNode(
  graphData: NormalizedGraphData,
  clusterLevels: GraphClusterLevel[],
  nodeId: string,
): ClusterPathItem[] {
  const nodeIndex = graphData.nodes.findIndex(node => node.id === nodeId)
  if (nodeIndex < 0) return []

  return [...clusterLevels]
    .sort((a, b) => b.level - a.level)
    .map(level => ({ level: level.level, clusterId: level.membership[nodeIndex] }))
    .filter(item => item.clusterId !== undefined)
}

function buildClusterDrillGraph(
  graphData: NormalizedGraphData,
  clusterLevels: GraphClusterLevel[],
  clusterPath: ClusterPathItem[],
): NormalizedGraphData {
  const coarsestLevel = getCoarsestClusterLevel(clusterLevels)
  if (!coarsestLevel) return graphData

  const parent = clusterPath[clusterPath.length - 1]
  const currentLevelNumber = parent ? parent.level - 1 : coarsestLevel.level
  const nodeIndex = new Map(graphData.nodes.map((node, index) => [node.id, index]))
  const eligibleNodeIndexes = new Set<number>()

  graphData.nodes.forEach((node, index) => {
    if (isExpanderNode(node)) return
    if (nodeMatchesClusterPath(clusterLevels, index, clusterPath)) {
      eligibleNodeIndexes.add(index)
    }
  })

  if (currentLevelNumber < 0) {
    const finestLevel = getClusterLevel(clusterLevels, 0)
    const nodes = graphData.nodes.filter((node, index) => {
      if (isExpanderNode(node)) {
        return graphData.links.some(link => (
          (link.source === node.id && eligibleNodeIndexes.has(nodeIndex.get(link.target) ?? -1))
          || (link.target === node.id && eligibleNodeIndexes.has(nodeIndex.get(link.source) ?? -1))
        ))
      }
      return eligibleNodeIndexes.has(index)
    }).map((node, index) => {
      if (isExpanderNode(node)) return node
      const sourceIndex = nodeIndex.get(node.id) ?? index
      const clusterId = finestLevel?.membership[sourceIndex]
      return clusterId === undefined
        ? node
        : { ...node, community: clusterId, colorKey: `cluster:0:${clusterId}` }
    })
    const visibleNodeIds = new Set(nodes.map(node => node.id))
    const links = graphData.links.filter(link => (
      visibleNodeIds.has(link.source) && visibleNodeIds.has(link.target)
    ))

    return {
      nodes,
      links,
      csr: buildGraphCsr({ nodes, links }),
      clusterLevels: graphData.clusterLevels,
      clusterDebug: graphData.clusterDebug,
    }
  }

  const currentLevel = getClusterLevel(clusterLevels, currentLevelNumber)
  if (!currentLevel) return graphData

  const clusterCounts = new Map<number, number>()
  eligibleNodeIndexes.forEach(index => {
    const clusterId = currentLevel.membership[index]
    clusterCounts.set(clusterId, (clusterCounts.get(clusterId) || 0) + 1)
  })

  const nodes: GraphNode[] = currentLevel.clusters
    .filter(cluster => clusterCounts.has(cluster.clusterId))
    .filter(cluster => clusterBelongsToPath(cluster, clusterPath))
    .sort((a, b) => b.size - a.size || a.clusterId - b.clusterId)
    .map(cluster => ({
      id: getClusterNodeId(currentLevel.level, cluster.clusterId),
      name: cluster.label || `Cluster ${cluster.clusterId}`,
      label: currentLevel.level === coarsestLevel.level ? 'Cluster' : `Cluster L${currentLevel.level}`,
      community: cluster.clusterId,
      colorKey: `cluster:${currentLevel.level}:${cluster.clusterId}`,
      expansionKind: 'cluster',
      hiddenCount: clusterCounts.get(cluster.clusterId) ?? cluster.size,
    }))
  const visibleClusterIds = new Set(nodes.map(node => node.community).filter(id => id !== undefined))
  const visibleNodeIds = new Set(nodes.map(node => node.id))
  const edgeCounts = new Map<string, { source: string; target: string; labels: Map<string, number>; count: number }>()

  graphData.links.forEach(link => {
    const sourceIndex = nodeIndex.get(link.source)
    const targetIndex = nodeIndex.get(link.target)
    if (sourceIndex === undefined || targetIndex === undefined) return
    if (!eligibleNodeIndexes.has(sourceIndex) || !eligibleNodeIndexes.has(targetIndex)) return

    const sourceCluster = currentLevel.membership[sourceIndex]
    const targetCluster = currentLevel.membership[targetIndex]
    if (sourceCluster === targetCluster) return
    if (!visibleClusterIds.has(sourceCluster) || !visibleClusterIds.has(targetCluster)) return

    const source = getClusterNodeId(currentLevel.level, sourceCluster)
    const target = getClusterNodeId(currentLevel.level, targetCluster)
    if (!visibleNodeIds.has(source) || !visibleNodeIds.has(target)) return

    const key = `${source}\t${target}`
    const record = edgeCounts.get(key) || { source, target, labels: new Map<string, number>(), count: 0 }
    record.count += 1
    record.labels.set(link.label, (record.labels.get(link.label) || 0) + 1)
    edgeCounts.set(key, record)
  })

  const links = [...edgeCounts.values()].map(record => ({
    source: record.source,
    target: record.target,
    label: record.count === 1 ? [...record.labels.keys()][0] || 'edge' : `${record.count} edges`,
  }))

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

function ipcBytesToUint8Array(bytes: number[] | Uint8Array) {
  return bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes)
}

function arrowColumnToBigUint64Array(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  table: any,
  columnName: string,
): BigUint64Array {
  const column = table.getChild(columnName)
  if (!column) return new BigUint64Array()

  const values: bigint[] = []
  for (let index = 0; index < column.length; index += 1) {
    const value = column.get(index)
    if (value !== null && value !== undefined) {
      values.push(BigInt(value))
    }
  }
  return new BigUint64Array(values)
}

function decodeArrowCsr(graphData: NormalizedGraphData): GraphCsrArrays | null {
  if (!graphData.csrArrowIpc) return null

  try {
    const table = tableFromIPC(ipcBytesToUint8Array(graphData.csrArrowIpc))
    const indptr = arrowColumnToBigUint64Array(table, 'indptr')
    const indices = arrowColumnToBigUint64Array(table, 'indices')
    const edgeIds = arrowColumnToBigUint64Array(table, 'edge_ids')
    if (indptr.length === graphData.nodes.length + 1 && indices.length === graphData.links.length) {
      return {
        indptr,
        indices,
        edgeIds: edgeIds.length === graphData.links.length ? edgeIds : null,
      }
    }
  } catch (err) {
    console.warn('Failed to decode Arrow CSR, falling back to JSON CSR', err)
  }

  return null
}

function graphCsrArrays(graphData: NormalizedGraphData): GraphCsrArrays {
  const arrowCsr = decodeArrowCsr(graphData)
  if (arrowCsr) return arrowCsr

  const csr = buildGraphCsr(graphData)
  return {
    indptr: new BigUint64Array(csr.indptr.map(BigInt)),
    indices: new BigUint64Array(csr.indices.map(BigInt)),
    edgeIds: csr.edgeIds ? new BigUint64Array(csr.edgeIds.map(BigInt)) : null,
  }
}

function realNodeIds(nodes: GraphNode[]): Set<string> {
  return new Set(nodes.filter(node => !isExpanderNode(node) && !isClusterNode(node)).map(node => node.id))
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
  alwaysShow: boolean,
  textColor: string,
  backgroundColor: string,
) {
  if (!alwaysShow && edgeData.key !== hoveredEdgeId) return
  if (!edgeData.label) return

  const dx = targetData.x - sourceData.x
  const dy = targetData.y - sourceData.y
  const distance = Math.hypot(dx, dy)

  if (distance < 1) {
    const loopRadius = sourceData.size + 18
    const labelX = sourceData.x
    const labelY = sourceData.y - loopRadius - 8
    drawSigmaEdgeLabelBox(context, edgeData.label, labelX, labelY, 96, textColor, backgroundColor)
    return
  }

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

function drawSigmaEdgeLabelBox(
  context: CanvasRenderingContext2D,
  text: string,
  centerX: number,
  centerY: number,
  maxWidth: number,
  textColor: string,
  backgroundColor: string,
) {
  const fontSize = 11
  const paddingX = 5
  const paddingY = 3

  context.save()
  context.font = `600 ${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`
  context.textBaseline = 'middle'

  let label = text
  let textWidth = context.measureText(label).width
  if (textWidth > maxWidth) {
    while (label.length > 1 && context.measureText(`${label}...`).width > maxWidth) {
      label = label.slice(0, -1)
    }
    label = `${label}...`
    textWidth = context.measureText(label).width
  }

  const boxWidth = textWidth + paddingX * 2
  const boxHeight = fontSize + paddingY * 2
  const boxX = centerX - boxWidth / 2
  const boxY = centerY - boxHeight / 2

  context.fillStyle = backgroundColor
  drawRoundedRect(context, boxX, boxY, boxWidth, boxHeight, 4)
  context.fill()
  context.fillStyle = textColor
  context.fillText(label, boxX + paddingX, centerY)
  context.restore()
}

function getForceEndpointPosition(endpoint: ForceGraphLinkObject['source']): ForceGraphPosition | null {
  if (!endpoint || typeof endpoint !== 'object') return null
  if (typeof endpoint.x !== 'number' || typeof endpoint.y !== 'number') return null
  return endpoint as ForceGraphPosition
}

function drawForceSchemaLinkLabel(
  link: ForceGraphLinkObject,
  context: CanvasRenderingContext2D,
  globalScale: number,
  textColor: string,
  backgroundColor: string,
) {
  if (!link.label) return
  const source = getForceEndpointPosition(link.source)
  const target = getForceEndpointPosition(link.target)
  if (!source || !target) return

  const midX = (source.x + target.x) / 2
  const midY = (source.y + target.y) / 2
  const fontSize = Math.max(2.5, 12 / globalScale)
  const paddingX = 5 / globalScale
  const paddingY = 3 / globalScale

  context.save()
  context.font = `600 ${fontSize}px system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`
  context.textAlign = 'center'
  context.textBaseline = 'middle'

  const textWidth = context.measureText(link.label).width
  const boxWidth = textWidth + paddingX * 2
  const boxHeight = fontSize + paddingY * 2
  const boxX = midX - boxWidth / 2
  const boxY = midY - boxHeight / 2

  context.fillStyle = backgroundColor
  drawRoundedRect(context, boxX, boxY, boxWidth, boxHeight, 4 / globalScale)
  context.fill()
  context.fillStyle = textColor
  context.fillText(link.label, midX, midY)
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

function SigmaGraphView({ graphData, labelNodeIds, newlyExpandedNodeIds, darkMode, alwaysShowEdgeLabels, getNodeDisplayName, getNodeColor, getNodeSize, getEdgeColor, onNodeClick }: SigmaGraphViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const rendererRef = useRef<Sigma | null>(null)
  const hoveredEdgeRef = useRef<string | null>(null)

  const graph = useMemo(() => {
    const { positions } = createInitialLayout(graphData)
    const nodes = graphData.nodes.map(node => {
      const position = positions[node.id] || { x: 0, y: 0 }
      const displayName = getNodeDisplayName(node)
      return {
        key: node.id,
        attributes: {
          x: position.x,
          y: position.y,
          size: getNodeSize(node),
          color: isExpanderNode(node) ? '#f59e0b' : getNodeColor(node),
          label: isExpanderNode(node) || labelNodeIds.has(node.id) ? displayName : '',
          hoverLabel: displayName,
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
    const csr = graphCsrArrays(graphData)

    return new IcebugSigmaGraph<SigmaNodeAttributes, SigmaEdgeAttributes>({
      directed: true,
      nodes,
      csr: {
        indptr: csr.indptr,
        indices: csr.indices,
        edgeIds: csr.edgeIds,
      },
      edgeAttributes,
      edgeKeys,
    })
  }, [graphData, labelNodeIds, newlyExpandedNodeIds, getNodeDisplayName, getNodeColor, getNodeSize, getEdgeColor])

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
          alwaysShowEdgeLabels,
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
  }, [graph, darkMode, alwaysShowEdgeLabels, onNodeClick])

  return <div ref={containerRef} className="sigma-canvas" />
}

function App() {
  const [databases, setDatabases] = useState<Database[]>([])
  const [selectedId, setSelectedId] = useState(0)
  const [graphData, setGraphData] = useState<GraphData>({ nodes: [], links: [] })
  const [schemaGraphData, setSchemaGraphData] = useState<GraphData>({ nodes: [], links: [] })
  const [schemaGraphDatabaseId, setSchemaGraphDatabaseId] = useState<number | null>(null)
  const [graphViewMode, setGraphViewMode] = useState<GraphViewMode>('data')
  const [loading, setLoading] = useState(false)
  const [schemaLoading, setSchemaLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [schemaError, setSchemaError] = useState<string | null>(null)
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
  const [clusterPath, setClusterPath] = useState<ClusterPathItem[]>([])
  const [clusterViewEnabled, setClusterViewEnabled] = useState(true)
  const [nodeSearch, setNodeSearch] = useState('')
  const [searchResults, setSearchResults] = useState<GraphNode[]>([])
  const [displayColumnsByLabel, setDisplayColumnsByLabel] = useState<Record<string, string>>({})
  const [searching, setSearching] = useState(false)
  const [searchError, setSearchError] = useState<string | null>(null)
  const [focusedNodeId, setFocusedNodeId] = useState<string | null>(null)
  const [lastExpandedNodeIds, setLastExpandedNodeIds] = useState<Set<string>>(() => new Set())
  const [llmSettingsOpen, setLlmSettingsOpen] = useState(false)
  const [llmClusterConfig, setLlmClusterConfig] = useState<LlmClusterNamingConfig>({
    accessToken: '',
    endpoint: 'https://openrouter.ai',
    model: 'deepseek-v4-flash',
    sampleSize: 15,
  })
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const graphRef = useRef<any>(null)
  const graphContainerRef = useRef<HTMLDivElement | null>(null)
  const [graphSize, setGraphSize] = useState({ width: 1, height: 1 })
  const customQueryRef = useRef<string>('')
  const llmClusterConfigRef = useRef<LlmClusterNamingConfig>(llmClusterConfig)
  const graphRequestInFlightRef = useRef<string | null>(null)
  const requestedClusterNameKeysRef = useRef<Set<string>>(new Set())
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const updateLlmClusterConfig = (patch: Partial<LlmClusterNamingConfig>) => {
    setLlmClusterConfig(current => {
      const next = { ...current, ...patch }
      llmClusterConfigRef.current = next
      return next
    })
  }

  const currentLlmClusterConfig = useCallback(() => buildLlmClusterConfig(llmClusterConfigRef.current), [])

  const resetClusterNameRequests = useCallback(() => {
    requestedClusterNameKeysRef.current.clear()
  }, [])

  const fetchDatabases = () => {
    Promise.all([
      invokeCommand<Database[]>('get_databases'),
      invokeCommand<number | null>('get_initial_database_id'),
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
    invokeCommand<{ current: string; parent: string; directories: { name: string; path: string; type: string }[]; files: { name: string; path: string; type: string }[] }>('get_directories', { path: dir || null })
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
      resetClusterNameRequests()
      setGraphData({ nodes: [], links: [] })
      return
    }
    setLoading(true)
    setError(null)

    const query = customQueryRef.current.trim()
    const llmConfig = currentLlmClusterConfig()
    const requestKey = JSON.stringify({
      id: selectedId,
      query,
      llmConfig,
    })
    if (graphRequestInFlightRef.current === requestKey) {
      console.info('Graph fetch skipped: identical request is already in flight', {
        id: selectedId,
        queryMode: query ? 'custom' : 'default',
        llmClusterNaming: Boolean(llmConfig),
        model: llmConfig?.model,
        endpoint: llmConfig?.endpoint,
      })
      return
    }
    graphRequestInFlightRef.current = requestKey
    const finishGraphFetch = () => {
      if (graphRequestInFlightRef.current === requestKey) {
        graphRequestInFlightRef.current = null
      }
      setLoading(false)
    }
    console.info('Graph fetch started', {
      id: selectedId,
      queryMode: query ? 'custom' : 'default',
      llmClusterNaming: Boolean(llmConfig),
      model: llmConfig?.model,
      endpoint: llmConfig?.endpoint,
    })
    if (query) {
      invokeCommand<GraphData>('execute_query', { id: selectedId, query, llmConfig })
        .then(data => {
          console.info('Graph cluster debug:', data.clusterDebug)
          resetClusterNameRequests()
          setGraphData(data)
          setLastExpandedNodeIds(new Set())
          setClusterPath([])
          setFocusedNodeId(null)
          finishGraphFetch()
          setTimeout(() => {
            if (graphRef.current) {
              graphRef.current.zoomToFit(400)
            }
          }, 500)
        })
        .catch(err => {
          console.error('Graph fetch failed', err)
          setError(String(err))
          finishGraphFetch()
        })
    } else {
      invokeCommand<GraphData>('get_graph', { id: selectedId, llmConfig })
        .then(data => {
          console.info('Graph cluster debug:', data.clusterDebug)
          resetClusterNameRequests()
          setGraphData(data)
          setLastExpandedNodeIds(new Set())
          setClusterPath([])
          setFocusedNodeId(null)
          finishGraphFetch()
          setTimeout(() => {
            if (graphRef.current) {
              graphRef.current.zoomToFit(400)
            }
          }, 500)
        })
        .catch(err => {
          console.error('Graph fetch failed', err)
          setError(String(err))
          finishGraphFetch()
        })
    }
  }, [selectedId, databases.length, currentLlmClusterConfig, resetClusterNameRequests])

  const fetchSchemaGraphData = useCallback((force = false) => {
    if (databases.length === 0) {
      setSchemaGraphData({ nodes: [], links: [] })
      setSchemaGraphDatabaseId(null)
      return
    }
    if (!force && schemaGraphDatabaseId === selectedId && schemaGraphData.nodes.length > 0) {
      return
    }

    setSchemaLoading(true)
    setSchemaError(null)
    invokeCommand<GraphData>('get_schema_graph', { id: selectedId })
      .then(data => {
        setSchemaGraphData(data)
        setSchemaGraphDatabaseId(selectedId)
        setSchemaLoading(false)
      })
      .catch(err => {
        setSchemaError(String(err))
        setSchemaGraphData({ nodes: [], links: [] })
        setSchemaGraphDatabaseId(null)
        setSchemaLoading(false)
      })
  }, [databases.length, schemaGraphData.nodes.length, schemaGraphDatabaseId, selectedId])

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
      await invokeCommand('add_database', { filePath })
      fetchDatabases()
      setFilePickerOpen(false)
      setPickerError(null)
      setManualPath('')
    } catch (err) {
      setPickerError(String(err))
    }
  }

  const runNodeSearch = useCallback(() => {
    const query = nodeSearch.trim()
    if (!query) {
      setSearchResults([])
      setSearchError(null)
      return
    }

    setSearching(true)
    setSearchError(null)
    invokeCommand<GraphNode[]>('search_nodes', { id: selectedId, query })
      .then(results => {
        setSearchResults(results)
        setSearching(false)
      })
      .catch(err => {
        setSearchError(String(err))
        setSearchResults([])
        setSearching(false)
      })
  }, [nodeSearch, selectedId])

  const exploreSearchResult = useCallback((node: GraphNode) => {
    setLoading(true)
    setError(null)
    setSearchError(null)
    invokeCommand<GraphData>('get_node_neighborhood', {
      id: selectedId,
      nodeId: node.id,
      llmConfig: currentLlmClusterConfig(),
    })
      .then(data => {
        const normalized = normalizeGraphData(data)
        const levels = buildCommunityClusterLevels(normalized)
        resetClusterNameRequests()
        setGraphData(data)
        setClusterViewEnabled(levels.length > 0)
        setClusterPath(getClusterPathForNode(normalized, levels, node.id))
        setFocusedNodeId(node.id)
        setLastExpandedNodeIds(new Set([node.id]))
        setLoading(false)
      })
      .catch(err => {
        setError(String(err))
        setLoading(false)
      })
  }, [selectedId, currentLlmClusterConfig, resetClusterNameRequests])

  const colorMapRef = useRef<Record<string, string>>({})
  const edgeColorMapRef = useRef<Record<string, string>>({})

  const handleNodeClick = useCallback((nodeId: string) => {
    const node = graphData.nodes.find(item => item.id === nodeId)
    if (!node?.expansionKind || node.expansionKind !== 'node' || !node.expandNodeId) return
    const expandedNodeId = node.expandNodeId

    setLoading(true)
    setError(null)
    invokeCommand<GraphData>('expand_node', {
      id: selectedId,
      nodeId: expandedNodeId,
      visibleNodeIds: graphData.nodes.map(item => item.id),
      offset: node.offset ?? 0,
      llmConfig: currentLlmClusterConfig(),
    })
      .then(data => {
        const beforeNodeIds = realNodeIds(graphData.nodes)
        const highlightedNodeIds = realNodeIds(data.nodes)
        const normalized = normalizeGraphData(data)
        const levels = buildCommunityClusterLevels(normalized)

        beforeNodeIds.forEach(id => {
          highlightedNodeIds.delete(id)
        })
        resetClusterNameRequests()
        setGraphData(data)
        setLastExpandedNodeIds(highlightedNodeIds)
        setClusterPath(getClusterPathForNode(normalized, levels, expandedNodeId))
        setFocusedNodeId(expandedNodeId)
        setLoading(false)
      })
      .catch(err => {
        setError(String(err))
        setLoading(false)
      })
  }, [graphData, selectedId, currentLlmClusterConfig, resetClusterNameRequests])

  const normalizedGraphData = useMemo(() => normalizeGraphData(graphData), [graphData])
  const normalizedSchemaGraphData = useMemo(() => normalizeGraphData(schemaGraphData), [schemaGraphData])
  const displayColumnOptions = useMemo(() => {
    const columnsByLabel = new Map<string, Set<string>>()
    graphData.nodes.forEach(node => {
      if (isExpanderNode(node) || isClusterNode(node)) return
      const propertyNames = Object.keys(node.properties || {})
      if (propertyNames.length === 0) return
      const columns = columnsByLabel.get(node.label) || new Set<string>()
      propertyNames.forEach(name => columns.add(name))
      columnsByLabel.set(node.label, columns)
    })

    return [...columnsByLabel.entries()]
      .map(([label, columns]) => ({
        label,
        columns: [...columns].sort((a, b) => a.localeCompare(b)),
      }))
      .sort((a, b) => a.label.localeCompare(b.label))
  }, [graphData.nodes])
  const getDisplayName = useCallback((node: GraphNode) => (
    getNodeDisplayName(node, displayColumnsByLabel)
  ), [displayColumnsByLabel])
  const clusterLevels = useMemo(() => buildCommunityClusterLevels(normalizedGraphData), [normalizedGraphData])
  const coarsestClusterLevel = useMemo(() => getCoarsestClusterLevel(clusterLevels), [clusterLevels])
  const currentClusterLevel = useMemo(() => {
    const parent = clusterPath[clusterPath.length - 1]
    const currentLevelNumber = parent ? parent.level - 1 : coarsestClusterLevel?.level
    return currentLevelNumber === undefined ? null : getClusterLevel(clusterLevels, currentLevelNumber)
  }, [clusterLevels, clusterPath, coarsestClusterLevel])
  const clusterBreadcrumbs = useMemo(() => clusterPath.map(pathItem => {
    const level = getClusterLevel(clusterLevels, pathItem.level)
    const cluster = level?.clusters.find(item => item.clusterId === pathItem.clusterId)
    return {
      ...pathItem,
      label: cluster?.label || `Cluster ${pathItem.clusterId}`,
      size: cluster?.size,
    }
  }), [clusterLevels, clusterPath])
  const visibleClusterPath = useMemo(() => (
    clusterPath.filter(pathItem => getClusterLevel(clusterLevels, pathItem.level))
  ), [clusterLevels, clusterPath])
  const visibleGraphData = useMemo(() => (
    clusterViewEnabled && clusterLevels.length > 0
      ? buildClusterDrillGraph(normalizedGraphData, clusterLevels, visibleClusterPath)
      : normalizedGraphData
  ), [clusterViewEnabled, clusterLevels, normalizedGraphData, visibleClusterPath])
  const visibleSchemaGraphData = useMemo(() => (
    expandSchemaSelfLinks(normalizedSchemaGraphData)
  ), [normalizedSchemaGraphData])
  const displayedGraphData = graphViewMode === 'schema' ? visibleSchemaGraphData : visibleGraphData
  const displayedGraphSourceData = graphViewMode === 'schema' ? schemaGraphData : graphData
  const displayedLoading = graphViewMode === 'schema' ? schemaLoading : loading
  const displayedError = graphViewMode === 'schema' ? schemaError : error

  useEffect(() => {
    resetClusterNameRequests()
  }, [displayColumnsByLabel, resetClusterNameRequests])

  useEffect(() => {
    const llmConfig = currentLlmClusterConfig()
    if (!llmConfig || !clusterViewEnabled || !currentClusterLevel) return

    const visibleClusterIds = visibleGraphData.nodes
      .filter(isClusterNode)
      .map(node => node.community)
      .filter((clusterId): clusterId is number => clusterId !== undefined)
    if (visibleClusterIds.length === 0) return

    const requests: LlmClusterNameRequest[] = []
    const currentPathKey = visibleClusterPath.map(item => `${item.level}:${item.clusterId}`).join('/')

    visibleClusterIds.forEach(clusterId => {
      const key = `${currentClusterLevel.level}:${clusterId}:${currentPathKey}`
      if (requestedClusterNameKeysRef.current.has(key)) return

      const labels = normalizedGraphData.nodes
        .filter((node, index) => (
          !isExpanderNode(node)
          && currentClusterLevel.membership[index] === clusterId
          && nodeMatchesClusterPath(clusterLevels, index, visibleClusterPath)
        ))
        .map(node => getNodeClusterLabel(node, displayColumnsByLabel))
      const sampledLabels = sampleLabels(labels, llmConfig.sampleSize)
      if (sampledLabels.length === 0) return

      requests.push({
        key,
        clusterId,
        labels: sampledLabels,
      })
    })

    if (requests.length === 0) return
    const cappedRequests = requests.slice(0, 24)
    requests.forEach(request => {
      requestedClusterNameKeysRef.current.add(request.key)
    })
    console.info('Visible cluster naming started', {
      requested: requests.length,
      sent: cappedRequests.length,
      level: currentClusterLevel.level,
      path: currentPathKey || 'root',
      model: llmConfig.model,
      endpoint: llmConfig.endpoint,
    })

    invokeCommand<LlmClusterNameResult[]>('name_visible_clusters', {
      llmConfig,
      clusters: cappedRequests,
    })
      .then(results => {
        const namesByKey = new Map(
          results
            .filter(result => result.name)
            .map(result => [result.key, result.name as string]),
        )
        console.info('Visible cluster naming finished', {
          requested: cappedRequests.length,
          named: namesByKey.size,
          errors: results.filter(result => result.error).length,
        })
        if (namesByKey.size === 0) return

        setGraphData(current => ({
          ...current,
          clusterLevels: current.clusterLevels?.map(level => (
            level.level !== currentClusterLevel.level
              ? level
              : {
                  ...level,
                  clusters: level.clusters.map(cluster => {
                    const key = `${level.level}:${cluster.clusterId}:${currentPathKey}`
                    const name = namesByKey.get(key)
                    return name ? { ...cluster, label: name } : cluster
                  }),
                }
          )),
        }))
      })
      .catch(err => {
        console.error('Visible cluster naming failed', err)
      })
  }, [
    clusterLevels,
    clusterViewEnabled,
    currentClusterLevel,
    currentLlmClusterConfig,
    displayColumnsByLabel,
    normalizedGraphData,
    visibleClusterPath,
    visibleGraphData.nodes,
  ])

  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    if (clusterLevels.length === 0) {
      setClusterPath([])
      return
    }
    if (visibleClusterPath.length !== clusterPath.length) {
      setClusterPath(visibleClusterPath)
    }
  }, [clusterLevels.length, clusterPath, visibleClusterPath])
  /* eslint-enable react-hooks/set-state-in-effect */

  const forceGraphData = useMemo<ForceGraphData<GraphNode, ForceGraphLink>>(() => ({
    nodes: displayedGraphData.nodes.map(node => ({ ...node })),
    links: displayedGraphData.links.map(link => ({ ...link })),
  }), [displayedGraphData])

  useEffect(() => {
    if (renderer !== 'force' || !graphRef.current) return
    const linkForce = graphRef.current.d3Force('link') as {
      distance?: (distance: number | ((link: ForceGraphLinkObject) => number)) => unknown
    } | undefined
    linkForce?.distance?.(graphViewMode === 'schema' ? 150 : 40)
    graphRef.current.d3ReheatSimulation?.()
  }, [renderer, graphViewMode, forceGraphData])

  const nodeDegree = useMemo(() => {
    const degrees: Record<string, number> = {}
    displayedGraphData.nodes.forEach(n => degrees[n.id] = 0)
    displayedGraphData.links.forEach(link => {
      degrees[link.source] = (degrees[link.source] || 0) + 1
      degrees[link.target] = (degrees[link.target] || 0) + 1
    })
    return degrees
  }, [displayedGraphData])

  const maxDegree = useMemo(() => Math.max(1, ...Object.values(nodeDegree)), [nodeDegree])

  const topLabelNodeIds = useMemo(() => {
    return new Set(
      [
        ...lastExpandedNodeIds,
        ...[...displayedGraphData.nodes]
        .sort((a, b) => (nodeDegree[b.id] || 0) - (nodeDegree[a.id] || 0))
        .filter(node => !isExpanderNode(node))
        .slice(0, graphViewMode === 'schema' ? 12 : 5)
        .map(node => node.id),
        ...(focusedNodeId ? [focusedNodeId] : []),
      ]
    )
  }, [lastExpandedNodeIds, displayedGraphData.nodes, nodeDegree, focusedNodeId, graphViewMode])

  const getNodeColor = useCallback((node: GraphNode) => {
    const key = node.colorKey || (node.community === undefined ? node.label : `${node.label}:${node.community}`)
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
    if (node.expansionKind === 'cluster') {
      return Math.min(34, 5 + Math.sqrt(node.hiddenCount || 1) * 3)
    }
    const degree = nodeDegree[node.id] || 0
    return 4 + (degree / maxDegree) * 12
  }, [nodeDegree, maxDegree])

  const handleVisibleNodeClick = useCallback((nodeId: string) => {
    if (graphViewMode === 'schema') return
    const clusterNode = parseClusterNodeId(nodeId)
    if (clusterNode) {
      setClusterViewEnabled(true)
      setClusterPath(path => {
        const parentIndex = path.findIndex(item => item.level <= clusterNode.level)
        const basePath = parentIndex === -1 ? path : path.slice(0, parentIndex)
        return [...basePath, clusterNode]
      })
      return
    }
    handleNodeClick(nodeId)
  }, [graphViewMode, handleNodeClick])

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

    const displayName = getDisplayName(node)
    if ((isExpanderNode(node) || topLabelNodeIds.has(node.id)) && displayName) {
      const fontSize = 3
      ctx.font = `${fontSize}px Sans-Serif`
      ctx.textAlign = 'center'
      ctx.textBaseline = 'middle'
      ctx.fillStyle = highlighted ? '#f59e0b' : '#fff'

      const maxWidth = size * 1.6
      let label = displayName
      const measured = ctx.measureText(label)
      if (measured.width > maxWidth) {
        while (label.length > 1 && ctx.measureText(label + '\u2026').width > maxWidth) {
          label = label.slice(0, -1)
        }
        label = label + '\u2026'
      }
      ctx.fillText(label, node.x, node.y)
    }
  }, [getNodeSize, getNodeColor, getDisplayName, darkMode, topLabelNodeIds, lastExpandedNodeIds])

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
                    setGraphViewMode('data')
                    setSchemaError(null)
                    setNodeSearch('')
                    setSearchResults([])
                    setSearchError(null)
                    setFocusedNodeId(null)
                  }}
                  title={db.relativePath}
                >
                  {db.name}
                </li>
              ))}
            </ul>
          )}
          {displayColumnOptions.length > 0 && (
            <div className="display-column-settings">
              <div className="panel-title">Node Labels</div>
              <div className="display-column-list">
                {displayColumnOptions.map(({ label, columns }) => {
                  const selectedColumn = displayColumnsByLabel[label]
                  const selectedValue = selectedColumn && columns.includes(selectedColumn)
                    ? selectedColumn
                    : AUTO_DISPLAY_COLUMN

                  return (
                    <label className="display-column-row" key={label}>
                      <span title={label}>{label}</span>
                      <select
                        value={selectedValue}
                        onChange={event => {
                          const value = event.target.value
                          setDisplayColumnsByLabel(current => ({
                            ...current,
                            [label]: value,
                          }))
                        }}
                      >
                        <option value={AUTO_DISPLAY_COLUMN}>Auto</option>
                        {columns.map(column => (
                          <option key={column} value={column}>{column}</option>
                        ))}
                      </select>
                    </label>
                  )
                })}
              </div>
            </div>
          )}
          <div className="node-search">
            <div className="panel-title">Find Node</div>
            <div className="node-search-row">
              <input
                value={nodeSearch}
                placeholder="Search name, label, or id"
                onChange={event => setNodeSearch(event.target.value)}
                onKeyDown={event => {
                  if (event.key === 'Enter') {
                    runNodeSearch()
                  }
                }}
              />
              <button onClick={runNodeSearch} disabled={searching || !nodeSearch.trim()}>
                {searching ? '...' : 'Go'}
              </button>
            </div>
            {searchError && <div className="search-error">{searchError}</div>}
            {searchResults.length > 0 && (
              <div className="search-results">
                {searchResults.map(result => (
                  <button
                    key={result.id}
                    className="search-result"
                    onClick={() => exploreSearchResult(result)}
                    title={`${result.label}: ${getDisplayName(result)}`}
                  >
                    <span>{getDisplayName(result)}</span>
                    <small>{result.label} · {result.id}</small>
                  </button>
                ))}
              </div>
            )}
          </div>
          <div className="llm-cluster-settings">
            <button
              className="llm-cluster-summary"
              onClick={() => setLlmSettingsOpen(open => !open)}
            >
              <span>Cluster Names</span>
              <small>{llmClusterConfig.accessToken.trim() ? 'LLM on' : 'Local'}</small>
            </button>
            {llmSettingsOpen && (
              <div className="llm-cluster-fields">
                <input
                  type="password"
                  value={llmClusterConfig.accessToken}
                  placeholder="LLM access token"
                  autoComplete="off"
                  onChange={event => updateLlmClusterConfig({ accessToken: event.target.value })}
                />
                <input
                  value={llmClusterConfig.model || ''}
                  placeholder="Model"
                  onChange={event => updateLlmClusterConfig({ model: event.target.value })}
                />
                <input
                  value={llmClusterConfig.endpoint || ''}
                  placeholder="OpenAI-compatible endpoint"
                  onChange={event => updateLlmClusterConfig({ endpoint: event.target.value })}
                />
              </div>
            )}
            <div className="llm-cluster-actions">
              <span>{llmClusterConfig.accessToken.trim() ? 'Visible only' : 'Using local names'}</span>
              <button
                onClick={() => {
                  const config = currentLlmClusterConfig()
                  console.info('Cluster naming apply clicked', {
                    llmClusterNaming: Boolean(config),
                    model: config?.model,
                    endpoint: config?.endpoint,
                  })
                  fetchGraphData()
                }}
                disabled={loading || databases.length === 0}
              >
                Apply
              </button>
            </div>
          </div>
        </div>
      </div>

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        <div className="header">
          <div className="header-left">
            <span className="graph-stats">
              {displayedLoading
                ? 'Loading...'
                : graphViewMode === 'schema'
                  ? `${schemaGraphData.nodes.length} schema tables, ${schemaGraphData.links.length} relationships`
                : clusterViewEnabled && clusterLevels.length > 0
                  ? currentClusterLevel
                    ? `${visibleGraphData.nodes.length} clusters at level ${currentClusterLevel.level}, ${visibleGraphData.links.length} aggregate edges`
                    : `${visibleGraphData.nodes.length} nodes in cluster, ${visibleGraphData.links.length} edges`
                  : `${graphData.nodes.length} nodes, ${graphData.links.length} edges`}
            </span>
            {graphViewMode === 'data' && focusedNodeId && <span className="focus-chip">Focused node {focusedNodeId}</span>}
            {displayedError && <span className="error-message">{displayedError}</span>}
          </div>

          <div className="header-right">
            {(clusterLevels.length > 0 || databases.length > 0) && (
              <div className="cluster-controls" aria-label="Clusters">
                {clusterLevels.length > 0 && (
                  <>
                    <button
                      className={graphViewMode === 'data' && clusterViewEnabled && clusterPath.length === 0 ? 'active' : ''}
                      onClick={() => {
                        setGraphViewMode('data')
                        setClusterViewEnabled(true)
                        setClusterPath([])
                      }}
                    >
                      Clusters
                    </button>
                    {clusterBreadcrumbs.map((item, index) => (
                      <button
                        key={`${item.level}:${item.clusterId}`}
                        className="breadcrumb-btn"
                        onClick={() => {
                          setGraphViewMode('data')
                          setClusterViewEnabled(true)
                          setClusterPath(clusterPath.slice(0, index + 1))
                        }}
                        title={item.size === undefined ? item.label : `${item.label}, ${item.size} nodes`}
                      >
                        {item.label}
                      </button>
                    ))}
                    <button
                      className={graphViewMode === 'data' && !clusterViewEnabled ? 'active' : ''}
                      onClick={() => {
                        setGraphViewMode('data')
                        setClusterViewEnabled(value => !value)
                      }}
                    >
                      {clusterViewEnabled ? 'All Nodes' : 'Cluster View'}
                    </button>
                  </>
                )}
                <button
                  className={graphViewMode === 'schema' ? 'active' : ''}
                  onClick={() => {
                    setGraphViewMode('schema')
                    fetchSchemaGraphData()
                  }}
                  disabled={schemaLoading || databases.length === 0}
                >
                  Schema
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
          {!displayedLoading && !displayedError && displayedGraphSourceData.nodes.length > 0 && renderer === 'sigma' && (
            <SigmaGraphView
              key={`${graphViewMode}-sigma`}
              graphData={displayedGraphData}
              labelNodeIds={topLabelNodeIds}
              newlyExpandedNodeIds={graphViewMode === 'schema' ? new Set() : lastExpandedNodeIds}
              darkMode={darkMode}
              alwaysShowEdgeLabels={graphViewMode === 'schema'}
              getNodeDisplayName={getDisplayName}
              getNodeColor={getNodeColor}
              getNodeSize={getNodeSize}
              getEdgeColor={getEdgeColor}
              onNodeClick={handleVisibleNodeClick}
            />
          )}
          {!displayedLoading && !displayedError && displayedGraphSourceData.nodes.length > 0 && renderer === 'force' && (
            <ForceGraph2D
              key={`${graphViewMode}-force`}
              ref={graphRef}
              width={graphSize.width}
              height={graphSize.height}
              graphData={forceGraphData}
              nodeCanvasObject={paintNode}
              onNodeClick={(node) => handleVisibleNodeClick(String(node.id))}
              nodeVal={(node) => { const s = getNodeSize(node); return s * s; }}
              nodeRelSize={1}
              nodeLabel={(node) => `${node.label}: ${getDisplayName(node)}`}
              linkLabel={(link) => link.label}
              linkColor={(link) => getEdgeColor(link.label)}
              linkCanvasObjectMode={() => graphViewMode === 'schema' ? 'after' : 'replace'}
              linkCanvasObject={(link, context, globalScale) => {
                if (graphViewMode !== 'schema') return
                drawForceSchemaLinkLabel(
                  link,
                  context,
                  globalScale,
                  '#111827',
                  darkMode ? 'rgba(248, 250, 252, 0.92)' : 'rgba(255, 255, 255, 0.92)',
                )
              }}
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
