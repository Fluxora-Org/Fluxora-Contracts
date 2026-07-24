const fs = require('fs');
const code = fs.readFileSync('contracts/stream/src/lib.rs', 'utf8');
const streamingMd = fs.readFileSync('docs/streaming.md', 'utf8');

const regex = /^\s*pub\s+fn\s+([a-zA-Z0-9_]+)\s*[\(<]/gm;
let m;
const entrypoints = new Set();
while ((m = regex.exec(code)) !== null) {
  entrypoints.add(m[1]);
}

console.log("Found entrypoints:", [...entrypoints]);

const allowlist = new Set(["save_stream", "require_not_paused", "require_not_globally_paused"]);

for (const ep of entrypoints) {
  if (allowlist.has(ep)) continue;
  if (!streamingMd.includes(ep)) {
    console.log(`MISSING DOC: '${ep}'`);
  }
}
