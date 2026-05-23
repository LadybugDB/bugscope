export interface SigmaSettings {
  allowInvalidContainer?: boolean
  defaultEdgeType?: string
  labelColor?: { color: string } | { attribute: string; color?: string }
  renderEdgeLabels?: boolean
  labelRenderedSizeThreshold?: number
  minCameraRatio?: number
  maxCameraRatio?: number
}

export class Sigma {
  constructor(graph: unknown, container: HTMLElement, settings?: SigmaSettings)
  refresh(): void
  kill(): void
}
