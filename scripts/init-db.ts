import { Database, Connection } from 'lbug';

const db = new Database(':memory:');
db.initSync();

const conn = new Connection(db);
conn.initSync();

await conn.query(`
  CREATE NODE TABLE Company (
    name STRING,
    valuation INT64,
    sector STRING
  )
`);

await conn.query(`
  CREATE NODE TABLE VC (
    name STRING,
    total_investment INT64
  )
`);

await conn.query(`
  CREATE REL TABLE INVESTED_IN (
    FROM VC TO Company,
    amount INT64
  )
`);

console.log('Database schema created');
conn.closeSync();
db.closeSync();
