import { Database, Connection } from 'lbug';
import fs from 'fs';

const dbPath = './data/companies.lbdb';

if (!fs.existsSync(dbPath)) {
  console.error(`Database not found at ${dbPath}. Run 'npx tsx scripts/load-data.ts' first.`);
  process.exit(1);
}

const db = new Database(dbPath);
db.initSync();
const conn = new Connection(db);
conn.initSync();

interface CompanyRow {
  name: string;
  valuation: number;
  sector: string;
}

interface VCRow {
  name: string;
  location: string;
  founded: number;
}

interface InvestmentRow {
  vc: string;
  company: string;
  amount: number;
}

const companiesResult = conn.querySync('MATCH (c:Company) RETURN c.name AS name, c.valuation AS valuation, c.sector AS sector') as any;
companiesResult.resetIterator();
const companies: CompanyRow[] = [];
let row;
while ((row = companiesResult.getNextSync()) !== null) {
  companies.push({ name: row.name, valuation: row.valuation, sector: row.sector });
}
companiesResult.close();

const vcsResult = conn.querySync('MATCH (v:VC) RETURN v.name AS name, v.location AS location, v.founded AS founded') as any;
vcsResult.resetIterator();
const vcs: VCRow[] = [];
while ((row = vcsResult.getNextSync()) !== null) {
  vcs.push({ name: row.name, location: row.location, founded: row.founded });
}
vcsResult.close();

const investmentsResult = conn.querySync('MATCH (v:VC)-[:INVESTED_IN]->(c:Company) RETURN v.name AS vc, c.name AS company, 1000000000 AS amount') as any;
investmentsResult.resetIterator();
const investments: InvestmentRow[] = [];
while ((row = investmentsResult.getNextSync()) !== null) {
  investments.push({ vc: row.vc, company: row.company, amount: row.amount });
}
investmentsResult.close();

conn.closeSync();
db.closeSync();

const output = {
  companies,
  vcs,
  investments
};

fs.writeFileSync('./src/db.json', JSON.stringify(output, null, 2));
console.log('Data exported to src/db.json');
console.log(`Companies: ${companies.length}, VCs: ${vcs.length}, Investments: ${investments.length}`);
