export interface SigmaSettings {
  allowInvalidContainer?: boolean
  defaultEdgeType?: string
  enableEdgeEvents?: boolean
  labelColor?: { color: string } | { attribute: string; color?: string }
  renderEdgeLabels?: boolean
  edgeLabelColor?: { color: string } | { attribute: string; color?: string }
  edgeLabelSize?: number
  edgeLabelWeight?: string
  labelRenderedSizeThreshold?: number
  minCameraRatio?: number
  maxCameraRatio?: number
  defaultDrawEdgeLabel?: (
    context: CanvasRenderingContext2D,
    edgeData: SigmaEdgeLabelData,
    sourceData: SigmaEdgeLabelNodeData,
    targetData: SigmaEdgeLabelNodeData,
  ) => void
  defaultDrawNodeLabel?: (context: CanvasRenderingContext2D, data: SigmaLabelData) => void
  defaultDrawNodeHover?: (context: CanvasRenderingContext2D, data: SigmaLabelData) => void
}

export interface SigmaEdgeLabelData {
  key?: string
  label?: string
  size: number
  forceLabel?: boolean
}

export interface SigmaEdgeLabelNodeData {
  x: number
  y: number
  size: number
}

export interface SigmaLabelData {
  x: number
  y: number
  size: number
  label?: string
  hoverLabel?: string
  color: string
  isNewlyExpanded?: boolean
}

export class Sigma {
  constructor(graph: unknown, container: HTMLElement, settings?: SigmaSettings)
  on(event: 'clickNode', callback: (payload: { node: string }) => void): void
  on(event: 'enterEdge', callback: (payload: { edge: string }) => void): void
  on(event: 'leaveEdge', callback: (payload: { edge: string }) => void): void
  refresh(): void
  kill(): void
}
