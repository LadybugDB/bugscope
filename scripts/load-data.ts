import { Database, Connection } from 'lbug';
import fs from 'fs';

const dbPath = './data/companies.lbdb';
fs.mkdirSync('./data', { recursive: true });

const db = new Database(dbPath);
db.initSync();

const conn = new Connection(db);
conn.initSync();

await conn.query(`
  CREATE NODE TABLE IF NOT EXISTS Company (
    name STRING PRIMARY KEY,
    valuation INT64,
    sector STRING
  )
`);

await conn.query(`
  CREATE NODE TABLE IF NOT EXISTS VC (
    name STRING PRIMARY KEY,
    location STRING,
    founded INT64
  )
`);

await conn.query(`
  CREATE REL TABLE IF NOT EXISTS INVESTED_IN (
    FROM VC TO Company,
    amount INT64
  )
`);

console.log('Database schema created');

const companyData = [
  { name: 'Google', valuation: 1700000000000, sector: 'Technology' },
  { name: 'Meta', valuation: 900000000000, sector: 'Technology' },
  { name: 'Stripe', valuation: 650000000000, sector: 'Fintech' },
  { name: 'Airbnb', valuation: 85000000000, sector: 'Travel' },
  { name: 'Uber', valuation: 120000000000, sector: 'Transportation' },
  { name: 'Snowflake', valuation: 600000000000, sector: 'Enterprise Software' },
  { name: 'Coinbase', valuation: 50000000000, sector: 'Crypto' },
  { name: 'Instacart', valuation: 130000000000, sector: 'E-commerce' },
  { name: 'DoorDash', valuation: 45000000000, sector: 'Food Delivery' },
  { name: 'Slack', valuation: 27000000000, sector: 'Enterprise Software' },
  { name: 'Twilio', valuation: 12000000000, sector: 'Enterprise Software' },
  { name: 'Zoom', valuation: 20000000000, sector: 'Enterprise Software' },
];

const vcData = [
  { name: 'Sequoia Capital', location: 'Menlo Park, CA', founded: 1972 },
  { name: 'Andreessen Horowitz', location: 'Menlo Park, CA', founded: 2009 },
  { name: 'Accel', location: 'Palo Alto, CA', founded: 1983 },
  { name: 'Greylock Partners', location: 'Menlo Park, CA', founded: 1965 },
  { name: 'Khosla Ventures', location: 'Menlo Park, CA', founded: 2004 },
  { name: 'Benchmark', location: 'San Francisco, CA', founded: 1995 },
  { name: 'Founders Fund', location: 'San Francisco, CA', founded: 2005 },
  { name: 'Union Square Ventures', location: 'New York, NY', founded: 2003 },
  { name: 'Lightspeed Venture Partners', location: 'Menlo Park, CA', founded: 2000 },
];

const investments = [
  { vc: 'Sequoia Capital', company: 'Google', amount: 12500000000 },
  { vc: 'Sequoia Capital', company: 'Stripe', amount: 2000000000 },
  { vc: 'Sequoia Capital', company: 'Airbnb', amount: 1500000000 },
  { vc: 'Sequoia Capital', company: 'Uber', amount: 1200000000 },
  { vc: 'Sequoia Capital', company: 'Slack', amount: 500000000 },
  { vc: 'Sequoia Capital', company: 'Instacart', amount: 1000000000 },
  { vc: 'Sequoia Capital', company: 'Twilio', amount: 300000000 },
  { vc: 'Andreessen Horowitz', company: 'Meta', amount: 500000000 },
  { vc: 'Andreessen Horowitz', company: 'Coinbase', amount: 250000000 },
  { vc: 'Andreessen Horowitz', company: 'Airbnb', amount: 550000000 },
  { vc: 'Andreessen Horowitz', company: 'Stripe', amount: 1000000000 },
  { vc: 'Andreessen Horowitz', company: 'Instacart', amount: 400000000 },
  { vc: 'Accel', company: 'Meta', amount: 1200000000 },
  { vc: 'Accel', company: 'Slack', amount: 160000000 },
  { vc: 'Accel', company: 'Snowflake', amount: 100000000 },
  { vc: 'Accel', company: 'Twilio', amount: 230000000 },
  { vc: 'Accel', company: 'Zoom', amount: 100000000 },
  { vc: 'Greylock Partners', company: 'Meta', amount: 250000000 },
  { vc: 'Greylock Partners', company: 'Airbnb', amount: 150000000 },
  { vc: 'Greylock Partners', company: 'DoorDash', amount: 400000000 },
  { vc: 'Khosla Ventures', company: 'Uber', amount: 260000000 },
  { vc: 'Khosla Ventures', company: 'Stripe', amount: 200000000 },
  { vc: 'Khosla Ventures', company: 'Instacart', amount: 300000000 },
  { vc: 'Benchmark', company: 'Uber', amount: 11000000 },
  { vc: 'Benchmark', company: 'DoorDash', amount: 160000000 },
  { vc: 'Founders Fund', company: 'Meta', amount: 500000000 },
  { vc: 'Founders Fund', company: 'Airbnb', amount: 300000000 },
  { vc: 'Founders Fund', company: 'Stripe', amount: 200000000 },
  { vc: 'Founders Fund', company: 'Uber', amount: 170000000 },
  { vc: 'Union Square Ventures', company: 'Snowflake', amount: 60000000 },
  { vc: 'Union Square Ventures', company: 'Coinbase', amount: 150000000 },
  { vc: 'Lightspeed Venture Partners', company: 'DoorDash', amount: 600000000 },
  { vc: 'Lightspeed Venture Partners', company: 'Instacart', amount: 400000000 },
];

for (const company of companyData) {
  await conn.query(
    `CREATE (c:Company {name: '${company.name}', valuation: ${company.valuation}, sector: '${company.sector}'});`
  );
}
console.log('Companies inserted');

for (const vc of vcData) {
  await conn.query(
    `CREATE (v:VC {name: '${vc.name}', location: '${vc.location}', founded: ${vc.founded}});`
  );
}
console.log('VCs inserted');

for (const inv of investments) {
  await conn.query(
    `MATCH (v:VC {name: '${inv.vc}'}), (c:Company {name: '${inv.company}'}) CREATE (v)-[:INVESTED_IN {amount: ${inv.amount}}]->(c);`
  );
}
console.log('Investments inserted');

conn.closeSync();
db.closeSync();

console.log('Database created at', dbPath);
