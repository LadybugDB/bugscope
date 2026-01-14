import { GraphVisualizer, GraphNode, GraphLink } from './graph';
import { loadGraphData, getNodeColor } from './data';
import * as d3 from 'd3';

console.log('=== Main script loaded ===');

document.addEventListener('DOMContentLoaded', async () => {
  console.log('DOM loaded');
  const container = document.getElementById('graph-container');
  if (!container) {
    console.error('Container not found');
    return;
  }

  try {
    const data = await loadGraphData();
    console.log('Data loaded:', data);

    // Create nodes array with both companies and VCs
    const nodes: GraphNode[] = [];

    // Scale for company radii
    const companyMinVal = Math.min(...data.companies.map(c => c.valuation));
    const companyMaxVal = Math.max(...data.companies.map(c => c.valuation));
    const companyRadiusScale = d3.scaleSqrt()
      .domain([companyMinVal, companyMaxVal])
      .range([25, 80]);

    // Add company nodes
    data.companies.forEach(c => {
      nodes.push({
        id: c.name,
        name: c.name,
        valuation: c.valuation,
        sector: c.sector,
        type: 'company',
        radius: companyRadiusScale(c.valuation),
        color: getNodeColor('Company', c.sector)
      });
    });

    // Scale for VC radii
    const vcMinInv = Math.min(...data.vcs.map(v => v.totalInvestment));
    const vcMaxInv = Math.max(...data.vcs.map(v => v.totalInvestment));
    const vcRadiusScale = d3.scaleSqrt()
      .domain([vcMinInv, vcMaxInv])
      .range([20, 60]);

    // Add VC nodes
    data.vcs.forEach(v => {
      nodes.push({
        id: v.name,
        name: v.name,
        valuation: 0,
        totalInvestment: v.totalInvestment,
        type: 'vc',
        radius: vcRadiusScale(v.totalInvestment),
        color: '#EA4335'
      });
    });

    // Create links from investments
    const links: GraphLink[] = data.investments.map(inv => ({
      source: inv.vc,
      target: inv.company,
      value: inv.amount
    }));

    console.log('Nodes:', nodes.length, 'Links:', links.length);

    const visualizer = new GraphVisualizer('graph-container');
    visualizer.render(nodes, links);
    console.log('=== Render called ===');
  } catch (error) {
    console.error('Error loading data:', error);
  }
});
