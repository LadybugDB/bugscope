import { Database, Connection } from 'lbug';
import path from 'path';
import fs from 'fs';

const dbPath = path.join(process.cwd(), 'data', 'companies.lbdb');

if (!fs.existsSync(dbPath)) {
  console.error(`Database not found at ${dbPath}. Run 'npx tsx scripts/load-data.ts' first.`);
  process.exit(1);
}

const db = new Database(dbPath, 0, false, true);
db.initSync();
const conn = new Connection(db);
conn.initSync();

const PORT = 3000;

function queryAllSync(query: string): any[] {
  const result = conn.querySync(query);
  const queryResult = result as any;
  queryResult.resetIterator();
  const rows: any[] = [];
  let row;
  while ((row = queryResult.getNextSync()) !== null) {
    rows.push(row);
  }
  queryResult.close();
  return rows;
}

Bun.serve({
  port: PORT,
  static: {
    "/": new Response(fs.readFileSync("./index.html"), { headers: { "Content-Type": "text/html" } }),
    "/main.js": new Response(fs.readFileSync("./dist/main.js"), { headers: { "Content-Type": "application/javascript" } }),
  },
  fetch(req) {
    const url = new URL(req.url);

    if (url.pathname === "/api/schema") {
      const schema = queryAllSync('CALL show_tables() RETURN *');
      return new Response(JSON.stringify(schema), { headers: { "Content-Type": "application/json" } });
    }

    if (url.pathname === "/api/columns") {
      const table = url.searchParams.get('table') || '';
      const columns = queryAllSync(`CALL table_info('${table}') RETURN *`);
      return new Response(JSON.stringify(columns), { headers: { "Content-Type": "application/json" } });
    }

    if (url.pathname === "/api/inspect") {
      const companyData = queryAllSync('MATCH (c:Company) RETURN c LIMIT 1');
      const vcData = queryAllSync('MATCH (v:VC) RETURN v LIMIT 1');
      return new Response(JSON.stringify({ company: companyData[0], vc: vcData[0] }, null, 2), { headers: { "Content-Type": "application/json" } });
    }

    if (url.pathname.startsWith('/api/query')) {
      const query = url.searchParams.get('q') || url.searchParams.get('query');
      if (!query) return new Response("Missing query parameter", { status: 400 });
      const results = queryAllSync(query);
      return new Response(JSON.stringify(results), { headers: { "Content-Type": "application/json" } });
    }

    return new Response("Not Found", { status: 404 });
  },
});

console.log(`Server running on http://localhost:${PORT}`);

process.on('SIGINT', () => {
  conn.closeSync();
  db.closeSync();
  process.exit(0);
});
