import { Database, Connection } from 'lbug';
import path from 'path';
import fs from 'fs';

const dbPath = path.join(process.cwd(), 'data', 'companies.lbdb');

if (!fs.existsSync(dbPath)) {
  console.error(`Database not found at ${dbPath}. Run 'npx tsx scripts/load-data.ts' first.`);
  process.exit(1);
}

const db = new Database(dbPath);
db.initSync();
const conn = new Connection(db);
conn.initSync();

const PORT = 3000;

Bun.serve({
  port: PORT,
  static: {
    "/": new Response(fs.readFileSync("./index.html"), { headers: { "Content-Type": "text/html" } }),
    "/main.js": new Response(fs.readFileSync("./dist/main.js"), { headers: { "Content-Type": "application/javascript" } }),
  },
  fetch(req) {
    const url = new URL(req.url);
    
    if (url.pathname === "/api/companies") {
      const result = conn.querySync('MATCH (c:Company) RETURN c.name AS name, c.valuation AS valuation, c.sector AS sector');
      const queryResult = result;
      queryResult.resetIterator();
      const companies = [];
      let row;
      while ((row = queryResult.getNextSync()) !== null) {
        companies.push(row);
      }
      queryResult.close();
      return new Response(JSON.stringify(companies), { headers: { "Content-Type": "application/json" } });
    }
    
    if (url.pathname === "/api/vcs") {
      const result = conn.querySync('MATCH (v:VC) RETURN v.name AS name, v.location AS location, v.founded AS founded');
      const queryResult = result;
      queryResult.resetIterator();
      const vcs = [];
      let row;
      while ((row = queryResult.getNextSync()) !== null) {
        vcs.push({ ...row, total_investment: row.founded * 1000000 });
      }
      queryResult.close();
      return new Response(JSON.stringify(vcs), { headers: { "Content-Type": "application/json" } });
    }
    
    if (url.pathname === "/api/investments") {
      const result = conn.querySync('MATCH (v:VC)-[r:INVESTED_IN]->(c:Company) RETURN v.name AS vc, c.name AS company, r.amount AS amount');
      const queryResult = result;
      queryResult.resetIterator();
      const investments = [];
      let row;
      while ((row = queryResult.getNextSync()) !== null) {
        investments.push(row);
      }
      queryResult.close();
      return new Response(JSON.stringify(investments), { headers: { "Content-Type": "application/json" } });
    }
    
    if (url.pathname === "/api/graph-data") {
      const companiesResult = conn.querySync('MATCH (c:Company) RETURN c.name AS name, c.valuation AS valuation, c.sector AS sector');
      companiesResult.resetIterator();
      const companies = [];
      let row;
      while ((row = companiesResult.getNextSync()) !== null) {
        companies.push({ ...row, type: 'company' });
      }
      companiesResult.close();

      // Query investments first
      const investmentsResult = conn.querySync('MATCH (v:VC)-[r:INVESTED_IN]->(c:Company) RETURN v.name AS vc, c.name AS company, r.amount AS amount');
      investmentsResult.resetIterator();
      const investments = [];
      const vcInvestmentMap = new Map();
      while ((row = investmentsResult.getNextSync()) !== null) {
        investments.push(row);
        const current = vcInvestmentMap.get(row.vc) || 0;
        vcInvestmentMap.set(row.vc, current + row.amount);
      }
      investmentsResult.close();

      // Query VCs and add totalInvestment calculated from investments
      const vcsResult = conn.querySync('MATCH (v:VC) RETURN v.name AS name, v.location AS location, v.founded AS founded');
      vcsResult.resetIterator();
      const vcs = [];
      while ((row = vcsResult.getNextSync()) !== null) {
        const totalInvestment = vcInvestmentMap.get(row.name) || 0;
        vcs.push({ ...row, type: 'vc', totalInvestment });
      }
      vcsResult.close();

      return new Response(JSON.stringify({ companies, vcs, investments }), { headers: { "Content-Type": "application/json" } });
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
