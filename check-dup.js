const fs = require('fs');
const code = fs.readFileSync('contracts/stream/src/lib.rs', 'utf8');

const errorBodyMatch = /pub\s+enum\s+ContractError\s*\{([^}]*)\}/m.exec(code);
if (errorBodyMatch) {
  const body = errorBodyMatch[1];
  const regex = /^\s{4}([A-Z][A-Za-z0-9]+)\s*=\s*(\d+)\s*,/gm;
  const seen = {};
  const exclude = new Set(["Operational", "Administrative", "Compliance", "Emergency", "GlobalEmergency"]);
  let m;
  while ((m = regex.exec(body)) !== null) {
    const variant = m[1];
    const val = m[2];
    if (exclude.has(variant)) continue;
    if (seen[val]) {
      console.log(`DUPLICATE DISCRIMINANT: '${variant}' and '${seen[val]}' both use value ${val}`);
    } else {
      seen[val] = variant;
    }
  }
}
