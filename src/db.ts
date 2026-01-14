import { Database, Connection } from 'lbug';
import fs from 'fs';

export interface SchemaInfo {
  tableName: string;
  columns: string[];
}

const dbPath = './data/companies.lbdb';

if (!fs.existsSync(dbPath)) {
  throw new Error(`Database not found at ${dbPath}. Run 'npx tsx scripts/load-data.ts' first.`);
}

const db = new Database(dbPath, 0, false, true);
db.initSync();
const conn = new Connection(db);
conn.initSync();

export function getSchema(): SchemaInfo[] {
  const result = conn.querySync('CALL show_tables() RETURN *') as any;
  result.resetIterator();

  const schema: SchemaInfo[] = [];
  let row;
  while ((row = result.getNextSync()) !== null) {
    schema.push({
      tableName: row.name,
      columns: []
    });
  }
  result.close();
  return schema;
}

export function getTableColumns(tableName: string): string[] {
  const result = conn.querySync(`CALL table_info('${tableName}') RETURN *`) as any;
  result.resetIterator();

  const columns: string[] = [];
  let row;
  while ((row = result.getNextSync()) !== null) {
    columns.push(row.name);
  }
  result.close();
  return columns;
}

export function getTableData(tableName: string): Record<string, any>[] {
  const result = conn.querySync(`MATCH (n:${tableName}) RETURN n`) as any;
  result.resetIterator();

  const data: Record<string, any>[] = [];
  let row;
  while ((row = result.getNextSync()) !== null) {
    data.push(row.n);
  }
  result.close();
  return data;
}

export function getRelationshipData(): Record<string, any>[] {
  const result = conn.querySync('MATCH (v:VC)-[r]->(c:Company) RETURN v, c, r') as any;
  result.resetIterator();

  const data: Record<string, any>[] = [];
  let row;
  while ((row = result.getNextSync()) !== null) {
    data.push({
      vc: row.v.name,
      company: row.c.name,
      amount: row.r?.amount || row.c.valuation || 1000000
    });
  }
  result.close();
  return data;
}

conn.closeSync();
db.closeSync();
