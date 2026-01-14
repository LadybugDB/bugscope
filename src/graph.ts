import * as d3 from 'd3';
import { CompanyData, VCData, getSectorColor, formatCurrency } from './data';

export interface GraphNode extends d3.SimulationNodeDatum {
  id: string;
  name: string;
  valuation: number;
  sector?: string;
  totalInvestment?: number;
  type: 'company' | 'vc';
  radius: number;
  color: string;
}

export interface GraphLink extends d3.SimulationLinkDatum<GraphNode> {
  source: GraphNode | string;
  target: GraphNode | string;
  value: number;
}

export class GraphVisualizer {
  private container: HTMLElement;
  private width: number;
  private height: number;
  private simulation: d3.Simulation<GraphNode, GraphLink>;
  private svg: d3.Selection<SVGSVGElement, unknown, null, undefined>;
  private nodes: d3.Selection<SVGGElement, GraphNode, SVGGElement, unknown>;
  private links: d3.Selection<SVGGElement, GraphLink, SVGGElement, unknown>;
  private tooltip: d3.Selection<HTMLDivElement, unknown, HTMLElement, undefined>;

  constructor(containerId: string) {
    this.container = document.getElementById(containerId)!;
    this.width = window.innerWidth;
    this.height = window.innerHeight;

    this.simulation = d3.forceSimulation<GraphNode>()
      .force('charge', d3.forceManyBody().strength(-200))
      .force('center', d3.forceCenter(this.width / 2, this.height / 2));

    this.svg = d3.select(this.container)
      .append('svg')
      .attr('width', this.width)
      .attr('height', this.height)
      .style('background-color', '#f8f9fa');

    // Order matters: links, nodes, then labels (drawn last = on top)
    this.links = this.svg.append('g').attr('class', 'links');
    this.nodes = this.svg.append('g').attr('class', 'nodes');
    const edgeLabelsGroup = this.svg.append('g').attr('class', 'edge-labels');
    this.svg.node()!.__edgeLabelsGroup__ = edgeLabelsGroup;

    this.tooltip = d3.select('body')
      .append('div')
      .attr('class', 'tooltip');

    window.addEventListener('resize', () => this.handleResize());
  }

  public render(nodes: GraphNode[], links: GraphLink[]): void {
    this.width = window.innerWidth;
    this.height = window.innerHeight;
    this.svg
      .attr('width', this.width)
      .attr('height', this.height);

    const sectorGroups = d3.group(nodes.filter(n => n.type === 'company'), d => d.sector!);
    const centerX = this.width / 2;
    const centerY = this.height / 2;
    const sectorCenters: Record<string, { x: number; y: number }> = {};
    const sectorEntries = Array.from(sectorGroups.entries());
    const totalSectors = sectorEntries.length + 1; // +1 for VC cluster
    const angleStep = (2 * Math.PI) / totalSectors;
    const orbitRadius = Math.min(this.width, this.height) * 0.25;

    sectorEntries.forEach(([sector], i) => {
      const angle = i * angleStep - Math.PI / 2;
      sectorCenters[sector] = {
        x: centerX + Math.cos(angle) * orbitRadius,
        y: centerY + Math.sin(angle) * orbitRadius,
      };
    });

    // Add VC cluster at the end
    const vcAngle = sectorEntries.length * angleStep - Math.PI / 2;
    sectorCenters['VC'] = {
      x: centerX + Math.cos(vcAngle) * orbitRadius,
      y: centerY + Math.sin(vcAngle) * orbitRadius,
    };

    // Assign initial positions
    nodes.forEach(node => {
      node.x = this.width / 2 + (Math.random() - 0.5) * 100;
      node.y = this.height / 2 + (Math.random() - 0.5) * 100;
    });

    // Find min/max for edge width scale
    const investmentAmounts = links.map(l => l.value);
    const minAmount = Math.min(...investmentAmounts);
    const maxAmount = Math.max(...investmentAmounts);
    const linkWidthScale = d3.scaleLinear()
      .domain([minAmount, maxAmount])
      .range([1, 8]);

    this.simulation
      .nodes(nodes)
      .force('link', d3.forceLink<GraphNode, GraphLink>(links)
        .id(d => d.id)
        .distance(150)
        .strength(0.3))
      .force('collision', d3.forceCollide<GraphNode>().radius(d => d.radius + 10).strength(0.9))
      .force('cluster', this.createClusterForce(sectorCenters, 0.3))
      .force('box', this.createBoundingBoxForce(100))
      .force('center', d3.forceCenter(this.width / 2, this.height / 2))
      .force('charge', d3.forceManyBody().strength(-300))
      .alpha(1)
      .restart();

    this.simulation.on('tick', () => this.tick());

    this.nodes.selectAll('g').remove();

    this.links.selectAll('line').remove();

    // Render links
    this.links.selectAll('line')
      .data(links)
      .join('line')
      .attr('stroke', '#999')
      .attr('stroke-opacity', 0.4)
      .attr('stroke-width', d => linkWidthScale(d.value));

    // Render edge labels in separate group (on top)
    const edgeLabelsGroup = (this.svg.node() as any).__edgeLabelsGroup__;
    edgeLabelsGroup.selectAll('g').remove();
    
    const labelGroups = edgeLabelsGroup.selectAll('g')
      .data(links)
      .join('g')
      .attr('class', 'edge-label');

    // Add background rectangle for each label
    labelGroups.append('rect')
      .attr('fill', 'white')
      .attr('fill-opacity', 0.85)
      .attr('rx', 3)
      .attr('ry', 3);

    // Add text on top of background
    labelGroups.append('text')
      .attr('text-anchor', 'middle')
      .attr('font-size', '11px')
      .attr('fill', '#333')
      .attr('font-weight', '600')
      .attr('dy', '0.35em')
      .attr('pointer-events', 'none')
      .text(d => {
        const amountInMillions = Math.round(d.value / 1000000);
        return amountInMillions >= 1000 
          ? `$${(amountInMillions / 1000).toFixed(1)}B` 
          : `$${amountInMillions}M`;
      })
      .each(function(d) {
        // Set rect size based on text bbox
        const bbox = (this as SVGTextElement).getBBox();
        d3.select((this as SVGTextElement).parentNode as SVGGElement).select('rect')
          .attr('x', bbox.x - 4)
          .attr('y', bbox.y - 2)
          .attr('width', bbox.width + 8)
          .attr('height', bbox.height + 4);
      });

    const nodeGroups = this.nodes.selectAll('g')
      .data(nodes)
      .join('g')
      .call((selection) => {
        const drag = d3.drag<SVGGElement, GraphNode>()
          .on('start', (event) => {
            if (!event.active) this.simulation.alphaTarget(0.3).restart();
            event.subject.fx = event.subject.x;
            event.subject.fy = event.subject.y;
          })
          .on('drag', (event) => {
            event.subject.fx = event.x;
            event.subject.fy = event.y;
          })
          .on('end', (event) => {
            if (!event.active) this.simulation.alphaTarget(0);
            event.subject.fx = null;
            event.subject.fy = null;
          });
        selection.call(drag);
      });

    nodeGroups.append('circle')
      .attr('r', d => d.radius)
      .attr('fill', d => d.color)
      .attr('stroke', '#fff')
      .attr('stroke-width', 2)
      .attr('cursor', 'pointer')
      .on('mouseover', (event, d) => {
        this.tooltip
          .classed('visible', true)
          .html(`
            <strong>${d.name}</strong><br/>
            ${d.type === 'company' 
              ? `Sector: ${d.sector}<br/>Valuation: ${formatCurrency(d.valuation)}`
              : `Total Investment: ${formatCurrency(d.totalInvestment!)}`
            }
          `)
          .style('left', (event.pageX + 10) + 'px')
          .style('top', (event.pageY - 10) + 'px');
        d3.select(event.currentTarget).attr('stroke', '#333');
      })
      .on('mousemove', (event) => {
        this.tooltip
          .style('left', (event.pageX + 10) + 'px')
          .style('top', (event.pageY - 10) + 'px');
      })
      .on('mouseout', (event) => {
        this.tooltip.classed('visible', false);
        d3.select(event.currentTarget).attr('stroke', '#fff');
      });

    nodeGroups.append('text')
      .text(d => d.name)
      .attr('text-anchor', 'middle')
      .attr('dy', '0.35em')
      .attr('font-size', d => Math.min(d.radius / 2.5, 14))
      .attr('font-weight', '600')
      .attr('fill', '#333')
      .style('pointer-events', 'none')
      .style('text-shadow', '1px 1px 2px rgba(255,255,255,0.8)');

    // Remove old legend
    this.svg.selectAll('.legend').remove();

    const legendContainer = this.svg.append('g')
      .attr('class', 'legend')
      .attr('transform', `translate(${this.width - 150}, 20)`);

    const sectors = Array.from(sectorGroups.keys());
    sectors.forEach((sector, i) => {
      const y = i * 25;
      const g = legendContainer.append('g')
        .attr('transform', `translate(0, ${y})`);

      g.append('circle')
        .attr('r', 8)
        .attr('fill', getSectorColor(sector));

      g.append('text')
        .attr('x', 15)
        .attr('y', 4)
        .text(sector)
        .attr('font-size', '12px')
        .attr('fill', '#333');
    });

    // Add VC legend item
    const vcY = sectors.length * 25;
    const vcG = legendContainer.append('g')
      .attr('transform', `translate(0, ${vcY})`);

    vcG.append('circle')
      .attr('r', 8)
      .attr('fill', '#EA4335');

    vcG.append('text')
      .attr('x', 15)
      .attr('y', 4)
      .text('VC')
      .attr('font-size', '12px')
      .attr('fill', '#333');
  }

  private createClusterForce(
    centers: Record<string, { x: number; y: number }>,
    strength: number
  ): d3.Force<GraphNode, GraphLink> {
    return (alpha) => {
      for (const node of this.simulation.nodes()!) {
        const clusterKey = node.type === 'vc' ? 'VC' : node.sector!;
        const center = centers[clusterKey];
        if (center) {
          node.vx! += (center.x - node.x!) * strength * alpha;
          node.vy! += (center.y - node.y!) * strength * alpha;
        }
      }
    };
  }

  private createBoundingBoxForce(padding: number): d3.Force<GraphNode, GraphLink> {
    return (alpha) => {
      for (const node of this.simulation.nodes()!) {
        if (node.x! < padding) {
          node.vx! += (padding - node.x!) * alpha;
        }
        if (node.x! > this.width - padding) {
          node.vx! += (this.width - padding - node.x!) * alpha;
        }
        if (node.y! < padding) {
          node.vy! += (padding - node.y!) * alpha;
        }
        if (node.y! > this.height - padding) {
          node.vy! += (this.height - padding - node.y!) * alpha;
        }
      }
    };
  }

  private tick(): void {
    this.links.selectAll('line')
      .attr('x1', d => (d.source as GraphNode).x!)
      .attr('y1', d => (d.source as GraphNode).y!)
      .attr('x2', d => (d.target as GraphNode).x!)
      .attr('y2', d => (d.target as GraphNode).y!);

    // Update edge label group positions
    const edgeLabelsGroup = (this.svg.node() as any).__edgeLabelsGroup__;
    edgeLabelsGroup.selectAll('.edge-label')
      .attr('transform', (d: GraphLink) => {
        const source = d.source as GraphNode;
        const target = d.target as GraphNode;
        const x = (source.x! + target.x!) / 2;
        const y = (source.y! + target.y!) / 2;
        return `translate(${x},${y})`;
      });

    this.nodes.selectAll('g')
      .attr('transform', d => `translate(${d.x},${d.y})`);
  }

  private handleResize(): void {
    this.width = window.innerWidth;
    this.height = window.innerHeight;
    this.svg
      .attr('width', this.width)
      .attr('height', this.height);
    this.simulation.force('center', d3.forceCenter(this.width / 2, this.height / 2));
    this.simulation.alpha(0.3).restart();
  }
}
