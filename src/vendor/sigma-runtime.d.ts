export interface SigmaSettings {
  allowInvalidContainer?: boolean
  defaultEdgeType?: string
  labelColor?: { color: string } | { attribute: string; color?: string }
  renderEdgeLabels?: boolean
  labelRenderedSizeThreshold?: number
  minCameraRatio?: number
  maxCameraRatio?: number
  defaultDrawNodeLabel?: (context: CanvasRenderingContext2D, data: SigmaLabelData) => void
  defaultDrawNodeHover?: (context: CanvasRenderingContext2D, data: SigmaLabelData) => void
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
  refresh(): void
  kill(): void
}
