export interface TableSchema {
  name: string;
  columns: string[];
}

export interface NodeTypeConfig {
  typeName: string;
  colorCategoryField: string | null;
  colorCategoryLabel: string | null;
}

export const NodeTypes: Record<string, NodeTypeConfig> = {
  Company: {
    typeName: 'Company',
    colorCategoryField: 'sector',
    colorCategoryLabel: 'sector'
  },
  VC: {
    typeName: 'VC',
    colorCategoryField: null,
    colorCategoryLabel: null
  }
};

export function setNodeTypes(config: Partial<Record<string, NodeTypeConfig>>): void {
  Object.assign(NodeTypes, config);
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
    if (table.name === NodeTypes.Company.typeName) {
      const data = await query(`MATCH (c:${table.name}) RETURN c`);
      companies.push(...data.map((row: any) => row.c));
    }
  }

  const relData = await query(`MATCH (v:${NodeTypes.VC.typeName})-[r]->(c:${NodeTypes.Company.typeName}) RETURN v, c, r`);
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

let cachedNodeColors: Record<string, Record<string, string>> | null = null;

export function getNodeColor(nodeType: string, category: string | null): string {
  if (cachedNodeColors) {
    return cachedNodeColors[nodeType]?.[category || ''] || '#CCCCCC';
  }

  cachedNodeColors = {};

  const palette = [
    '#FFB3BA', '#FFDFBA', '#FFFFBA', '#BAFFC9', '#BAE1FF',
    '#E6B3FF', '#B3E6FF', '#FFB3E6', '#B3FFBA', '#FFBAE6',
    '#BAFFFF', '#FFBAFF', '#E6FFBA', '#FFE6BA', '#BAE6FF'
  ];

  Object.keys(NodeTypes).forEach(type => {
    cachedNodeColors![type] = {};
    const config = NodeTypes[type];

    if (config.colorCategoryField) {
      const categories = new Set<string>();
      cachedData?.companies.forEach((company: any) => {
        const categoryValue = company[config.colorCategoryField];
        if (categoryValue) categories.add(categoryValue);
      });

      Array.from(categories).sort().forEach((cat, i) => {
        cachedNodeColors![type][cat] = palette[i % palette.length];
      });
    }
  });

  return cachedNodeColors[nodeType]?.[category || ''] || '#CCCCCC';
}

export function formatCurrency(value: number): string {
  if (value >= 1e12) return `$${(value / 1e12).toFixed(1)}T`;
  if (value >= 1e9) return `$${(value / 1e9).toFixed(1)}B`;
  if (value >= 1e6) return `$${(value / 1e6).toFixed(1)}M`;
  return `$${value}`;
}
