const fs = require('fs');

const code = fs.readFileSync('contracts/stream/src/lib.rs', 'utf8');
const errorBodyMatch = /pub\s+enum\s+ContractError\s*\{([\s\S]*?)^\}/m.exec(code);

let duplicateCount = 0;

if (errorBodyMatch) {
  const body = errorBodyMatch[1];
  const regex = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(\d+)\s*,/gm;
  const seen = {};
  let m;

  while ((m = regex.exec(body)) !== null) {
    const variant = m[1];
    const val = m[2];

    if (seen[val]) {
      duplicateCount += 1;
      console.error(
        `DUPLICATE DISCRIMINANT: '${variant}' and '${seen[val]}' both use value ${val}`,
      );
    } else {
      seen[val] = variant;
    }
  }
}

if (duplicateCount > 0) {
  process.exit(1);
}
