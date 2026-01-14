export interface TableSchema {
  name: string;
  columns: string[];
}

export interface GraphData {
  companies: Record<string, unknown>[];
  vcs: Record<string, unknown>[];
  investments: InvestmentData[];
}

export interface InvestmentData {
  vc: string;
  company: string;
  amount: number;
}

let cachedSchema: TableSchema[] | null = null;
let cachedData: GraphData | null = null;

export async function getSchema(): Promise<TableSchema[]> {
  if (cachedSchema) return cachedSchema;

  const response = await fetch('/api/schema');
  if (!response.ok) {
    throw new Error(`Failed to fetch schema: ${response.statusText}`);
  }

  const tables = await response.json();
  cachedSchema = await Promise.all(
    tables.map(async (table: { name: string }) => ({
      name: table.name,
      columns: await getTableColumns(table.name)
    }))
  );

  return cachedSchema;
}

async function getTableColumns(tableName: string): Promise<string[]> {
  const response = await fetch(`/api/columns?table=${encodeURIComponent(tableName)}`);
  if (!response.ok) return [];
  const columns = await response.json();
  return columns.map((c: { name: string }) => c.name);
}

export async function query(cypherQuery: string): Promise<Record<string, unknown>[]> {
  const response = await fetch(`/api/query?q=${encodeURIComponent(cypherQuery)}`);
  if (!response.ok) return [];
  return response.json();
}

export async function loadGraphData(): Promise<GraphData> {
  if (cachedData) return cachedData;

  const schema = await getSchema();
  const companies: Record<string, unknown>[] = [];

  for (const table of schema) {
    if (table.name === 'Company') {
      const data = await query(`MATCH (c:${table.name}) RETURN c`);
      companies.push(...data.map((row: any) => row.c));
    }
  }

  const relData = await query('MATCH (v:VC)-[r]->(c:Company) RETURN v, c, r');
  const vcInvestmentMap = new Map<string, number>();
  const vcs: Record<string, unknown>[] = [];

  relData.forEach((row: any) => {
    const current = vcInvestmentMap.get(row.v.name) || 0;
    vcInvestmentMap.set(row.v.name, current + (row.c.valuation || 1000000));
  });

  vcInvestmentMap.forEach((total, name) => {
    vcs.push({ name, totalInvestment: total });
  });

  const investments: InvestmentData[] = relData.map((row: any) => ({
    vc: row.v.name,
    company: row.c.name,
    amount: row.c.valuation || 1000000
  }));

  cachedData = { companies, vcs, investments };
  return cachedData;
}

export function getSectorColor(sector: string): string {
  const colors: Record<string, string> = {
    'Technology': '#FFB3BA',
    'Fintech': '#FFDFBA',
    'Travel': '#FFFFBA',
    'Transportation': '#BAFFC9',
    'Enterprise Software': '#BAE1FF',
    'Crypto': '#E6B3FF',
    'E-commerce': '#B3E6FF',
    'Food Delivery': '#FFB3E6',
  };
  return colors[sector] || '#CCCCCC';
}

export function formatCurrency(value: number): string {
  if (value >= 1e12) return `$${(value / 1e12).toFixed(1)}T`;
  if (value >= 1e9) return `$${(value / 1e9).toFixed(1)}B`;
  if (value >= 1e6) return `$${(value / 1e6).toFixed(1)}M`;
  return `$${value}`;
}
