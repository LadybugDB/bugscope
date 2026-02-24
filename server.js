import express from 'express'
import cors from 'cors'
import { Database, Connection } from 'lbug'
import path from 'path'
import { fileURLToPath } from 'url'
import fs from 'fs'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)

const app = express()
app.use(cors())
app.use(express.json())

const DATA_DIR = path.join(__dirname, '.')

function scanForDatabases(dir = DATA_DIR, baseDir = DATA_DIR) {
  const databases = []
  try {
    const entries = fs.readdirSync(dir, { withFileTypes: true })
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name)
      if (entry.isDirectory()) {
        databases.push(...scanForDatabases(fullPath, baseDir))
      } else if (entry.name.endsWith('.lbdb')) {
        const relativePath = path.relative(baseDir, fullPath)
        databases.push({
          name: entry.name.replace('.lbdb', ''),
          path: fullPath,
          relativePath
        })
      }
    }
  } catch (err) {
    console.error(`Error scanning ${dir}:`, err.message)
  }
  return databases
}

app.get('/api/databases', (req, res) => {
  const databases = scanForDatabases()
  res.json(databases.map((d, i) => ({ id: i, ...d })))
})

app.get('/api/graph/:id', async (req, res) => {
  const databases = scanForDatabases()
  const dbInfo = databases[parseInt(req.params.id)]
  if (!dbInfo) {
    return res.status(404).json({ error: 'Database not found' })
  }

  try {
    const db = new Database(dbInfo.path, 0, false, true)
    const conn = new Connection(db)
    
    const nodesResult = await conn.query('MATCH (n) RETURN n, LABEL(n) as label, ID(n) as nodeId LIMIT 500')
    const nodesRows = await nodesResult.getAll()
    
    const linksResult = await conn.query('MATCH (a)-[r]->(b) RETURN ID(a) as src, ID(b) as dst, LABEL(r) as relType LIMIT 500')
    const linksRows = await linksResult.getAll()
    
    const idToString = (id) => `${id.table}:${id.offset}`
    
    const nodes = nodesRows.map((row) => {
      const node = row.n || {}
      const label = row.label || 'Node'
      const name = node.name || node.id || node.title || 'Node'
      return { id: idToString(row.nodeId), name: String(name), label }
    })
    
    const nodeIdSet = new Set(nodes.map(n => n.id))
    
    const links = linksRows.map((row) => ({
      source: idToString(row.src),
      target: idToString(row.dst),
      label: row.relType
    })).filter((link) => nodeIdSet.has(link.source) && nodeIdSet.has(link.target))
    
    res.json({ nodes, links })
  } catch (err) {
    console.error(err)
    res.status(500).json({ error: err.message })
  }
})

const PORT = 3001
app.listen(PORT, () => {
  console.log(`Server running on http://localhost:${PORT}`)
})
