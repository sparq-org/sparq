// Research grade and not externally audited. Deterministic planner fixture.
const { createHash } = require('node:crypto');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const read = name => readFileSync(join(__dirname, name), 'utf8').trim();
exports.catalogueHash = () => read('approved-catalogue');
exports.toolchainPinsHash = () => sha256(readFileSync(join(__dirname, '../assets/toolchain.pins.json')));
exports.hostCall = (operation, payload) => {
  const input = JSON.parse(payload);
  if (operation === 'testIssuerKeygen') return JSON.stringify({ publicKey: 'fixture-issuer' });
  if (operation === 'request') return JSON.stringify(input);
  if (operation !== 'plan') throw new Error(`unexpected host operation ${operation}`);
  const planHash = sha256(input.query);
  const packageName = `query_${planHash}`;
  return JSON.stringify({
    planHash,
    packageName,
    files: {
      'Nargo.toml': packageName,
      'src/main.nr': read('scan-abi'),
    },
  });
};
