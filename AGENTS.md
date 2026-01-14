# Agent Notes

## Tools Used

### ladybug_query
Used to retrieve company data from LadybugDB.

**Initial Query:**
```cypher
MATCH (c:Company) RETURN c.name, c.total_investment, c.description LIMIT 10
```
Error: Property `total_investment` does not exist.

**Schema Discovery:**
```cypher
CALL show_tables() RETURN *
```
Result: Tables are `Company`, `VC`, `INVESTED_IN` (relationship).

**Property Discovery:**
```cypher
MATCH (c:Company) RETURN c LIMIT 1
```
Result: Properties are `name`, `valuation`, `sector`.

**Final Data Query:**
```cypher
MATCH (c:Company) RETURN c.name, c.valuation, c.sector LIMIT 50
```

**Retrieved Data:**
| Name | Valuation ($) | Sector |
|------|---------------|--------|
| Google | 1,700,000,000,000 | Technology |
| Meta | 900,000,000,000 | Technology |
| Stripe | 650,000,000,000 | Fintech |
| Airbnb | 85,000,000,000 | Travel |
| Uber | 120,000,000,000 | Transportation |
| Snowflake | 600,000,000,000 | Enterprise Software |
| Coinbase | 50,000,000,000 | Crypto |
| Instacart | 130,000,000,000 | E-commerce |
| DoorDash | 45,000,000,000 | Food Delivery |
| Slack | 27,000,000,000 | Enterprise Software |
| Twilio | 12,000,000,000 | Enterprise Software |
| Zoom | 20,000,000,000 | Enterprise Software |

## Implementation Details

- **Visualization**: D3.js force simulation with clustering by sector
- **Bubble Size**: Sqrt scale based on valuation (25px - 100px range)
- **Colors**: Pastel color palette assigned per sector
- **Text**: Legible labels inside bubbles, sized proportionally to radius
- **Interactions**: Drag nodes, hover tooltips
