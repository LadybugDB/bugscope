# Bugscope Graph Visualizer

An interactive graph visualization tool for ladybugdb. Explore relationships between bugs, files, and other entities in your databases through an intuitive visual interface.

## Features

- **Interactive Graph View** - Navigate through connected data using a force-directed graph. Drag nodes to rearrange, zoom in/out, and pan around the canvas.
- **Database Selection** - Choose from available ladybugdb databases in the sidebar to visualize their relationships.
- **Visual Encoding** - Node size reflects connection count (more connections = larger nodes), and colors differentiate entity types.
- **Dark/Light Mode** - Toggle between dark and light themes for comfortable viewing.
- **Relationship Labels** - Hover over edges to see the type of relationship between connected nodes.

## Getting Started

1. **Install dependencies**
   ```bash
   npm install
   ```

2. **Start the application**
   ```bash
   npm run dev
   ```

3. **Open in browser**
   Navigate to `http://localhost:5173` to view the visualizer.

The application will automatically connect to the backend API at `http://localhost:3001` to fetch available databases and graph data.

## Usage

1. Select a database from the sidebar on the left
2. The graph will load and display nodes (entities) and edges (relationships)
3. Click and drag nodes to rearrange the layout
4. Scroll to zoom in/out, click and drag the canvas to pan
5. Hover over nodes to see their labels
6. Hover over edges to see relationship types
7. Use the theme toggle button to switch between dark and light modes

## Requirements

- Node.js
- A running ladybugdb backend API at `http://localhost:3001`
