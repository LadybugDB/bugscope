# Bugscope Graph Visualizer

![Visualization Preview](assets/sample.png)

## Features

- **Bubble Chart Visualization**: Companies are displayed as bubbles with size proportional to their valuation
- **Color Coding**: Pastel colors by industry sector
- **VC Connections**: VCs are displayed in red with investment size shown on edges
- **Edge Width**: Edge width is determined by investment size
- **Interactive**: Drag bubbles, hover for tooltips
- **Responsive**: Adapts to window size
- **Force Layout**: Physics-based positioning with sector clustering

## Setup

1. Install dependencies:
   ```bash
   bun install
   ```

2. Start development server:
   ```bash
   bun run dev
   ```

3. Build for production:
   ```bash
   bun run build
   ```

## Data Source

Data is queried from `data/companies.lbdb` files via the API endpoint. The graph displays:
- Company name (inside bubble)
- Valuation (determines bubble size)
- Industry sector (determines color)

## Using with a Different Database

The visualization is configured for a graph database with `Company` and `VC` node types. To use with a different database schema, configure the `NodeTypes` object in `src/data.ts`:

```typescript
import { setNodeTypes, NodeTypes } from './data';

setNodeTypes({
  Company: {
    typeName: 'Organization',      // Cypher node label
    colorCategoryField: 'industry', // Property to color by
    colorCategoryLabel: 'industry'
  },
  VC: {
    typeName: 'Investor',
    colorCategoryField: null,       // No coloring for this type
    colorCategoryLabel: null
  }
});
```

This will:
1. Query `MATCH (n:Organization)` instead of `MATCH (c:Company)`
2. Color nodes by the `industry` property instead of `sector`
3. Query `MATCH (v:Investor)` instead of `MATCH (v:VC)`

The `colorCategoryField` property determines which node property is used for color assignment. Set it to `null` to use a single color for that node type.

## Project Structure

```
├── src/
│   ├── main.ts          # Entry point
│   ├── graph.ts         # D3.js visualization logic
│   └── data.ts          # Data handling and color schemes
├── index.html           # HTML template
├── package.json         # Dependencies and scripts
└── tsconfig.json        # TypeScript configuration
```
