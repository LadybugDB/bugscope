export interface CompanyData {
  name: string;
  valuation: number;
  sector: string;
}

export interface VCData {
  name: string;
  totalInvestment: number;
  location: string;
}

export interface InvestmentData {
  vc: string;
  company: string;
  amount: number;
}

export interface GraphData {
  companies: CompanyData[];
  vcs: VCData[];
  investments: InvestmentData[];
}

let cachedData: GraphData | null = null;

export async function loadGraphData(): Promise<GraphData> {
  if (cachedData) return cachedData;

  const response = await fetch('/api/graph-data');
  if (!response.ok) {
    throw new Error(`Failed to fetch data: ${response.statusText}`);
  }
  
  const data = await response.json();

  cachedData = {
    companies: data.companies,
    vcs: data.vcs,
    investments: data.investments
  };
  
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
