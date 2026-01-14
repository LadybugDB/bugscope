import { Database, Connection } from 'lbug';
import fs from 'fs';

export interface CompanyData {
  id: string;
  name: string;
  valuation: number;
  sector: string;
  type: 'company';
}

export interface VCData {
  id: string;
  name: string;
  totalInvestment: number;
  location: string;
  founded: number;
  type: 'vc';
}

export interface InvestmentData {
  from: string;
  to: string;
  amount: number;
}

const dbPath = './data/companies.lbdb';

if (!fs.existsSync(dbPath)) {
  throw new Error(`Database not found at ${dbPath}. Run 'npx tsx scripts/load-data.ts' first.`);
}

const db = new Database(dbPath);
db.initSync();
const conn = new Connection(db);
conn.initSync();

export function getCompanies(): CompanyData[] {
  const result = conn.querySync('MATCH (c:Company) RETURN c');
  const queryResult = result as any;
  queryResult.resetIterator();
  
  const companies: CompanyData[] = [];
  let row;
  while ((row = queryResult.getNextSync()) !== null) {
    const c = row.c as any;
    companies.push({
      id: `${c._id?.table}:${c._id?.offset}`,
      name: c.name,
      valuation: c.valuation,
      sector: c.sector,
      type: 'company'
    });
  }
  queryResult.close();
  return companies;
}

export function getVCs(): VCData[] {
  const result = conn.querySync('MATCH (v:VC) RETURN v');
  const queryResult = result as any;
  queryResult.resetIterator();
  
  const vcs: VCData[] = [];
  let row;
  while ((row = queryResult.getNextSync()) !== null) {
    const v = row.v as any;
    vcs.push({
      id: `${v._id?.table}:${v._id?.offset}`,
      name: v.name,
      totalInvestment: v.founded * 1000000,
      location: v.location,
      founded: v.founded,
      type: 'vc'
    });
  }
  queryResult.close();
  return vcs;
}

export function getInvestments(): InvestmentData[] {
  const result = conn.querySync('MATCH (v:VC)-[r:INVESTED_IN]->(c:Company) RETURN v.name AS vc, c.name AS company, r.amount AS amount');
  const queryResult = result as any;
  queryResult.resetIterator();
  
  const investments: InvestmentData[] = [];
  let row;
  while ((row = queryResult.getNextSync()) !== null) {
    investments.push({
      from: row.vc,
      to: row.company,
      amount: row.amount
    });
  }
  queryResult.close();
  return investments;
}

export function getAllData(): { companies: CompanyData[], vcs: VCData[], investments: InvestmentData[] } {
  return {
    companies: getCompanies(),
    vcs: getVCs(),
    investments: getInvestments()
  };
}

conn.closeSync();
db.closeSync();
