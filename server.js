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

let customDatabases = []

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

function getAllDatabases() {
  const scanned = scanForDatabases()
  const all = [...scanned, ...customDatabases]
  return all.map((d, i) => ({ id: i, ...d }))
}

app.get('/api/databases', (req, res) => {
  const databases = getAllDatabases()
  res.json(databases)
})

app.post('/api/databases', (req, res) => {
  const { filePath } = req.body
  if (!filePath) {
    return res.status(400).json({ error: 'filePath is required' })
  }

  try {
    const absPath = path.isAbsolute(filePath) ? filePath : path.resolve(filePath)
    if (!fs.existsSync(absPath)) {
      return res.status(404).json({ error: 'File not found' })
    }
    if (!absPath.endsWith('.lbdb')) {
      return res.status(400).json({ error: 'Only .lbdb files are supported' })
    }

    const existing = customDatabases.find(d => d.path === absPath)
    if (existing) {
      return res.status(409).json({ error: 'Database already added' })
    }

    const db = {
      name: path.basename(absPath, '.lbdb'),
      path: absPath,
      relativePath: absPath
    }
    customDatabases.push(db)
    res.json({ id: getAllDatabases().length - 1, ...db })
  } catch (err) {
    res.status(500).json({ error: err.message })
  }
})

app.get('/api/directories', (req, res) => {
  const dir = req.query.path ? path.resolve(req.query.path) : DATA_DIR
  try {
    const entries = fs.readdirSync(dir, { withFileTypes: true })
    const directories = entries
      .filter(e => e.isDirectory() && !e.name.startsWith('.'))
      .map(e => ({ name: e.name, path: path.join(dir, e.name), type: 'directory' }))
      .sort((a, b) => a.name.localeCompare(b.name))
    const files = entries
      .filter(e => e.isFile() && e.name.endsWith('.lbdb'))
      .map(e => ({ name: e.name.replace('.lbdb', ''), path: path.join(dir, e.name), type: 'file' }))
      .sort((a, b) => a.name.localeCompare(b.name))
    res.json({ current: dir, parent: path.dirname(dir), directories, files })
  } catch (err) {
    res.status(500).json({ error: err.message })
  }
})

app.get('/api/graph/:id', async (req, res) => {
  const databases = getAllDatabases()
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
