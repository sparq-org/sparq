const assert = require('node:assert/strict');
const Y = require('yjs');
const { WebsocketProvider } = require('y-websocket');
const WebSocket = require('ws');

const urls = JSON.parse(process.argv[2]);
const docs = urls.map(() => new Y.Doc());
const expectedCloses = new Set();
const unexpectedCloses = [];
setTimeout(() => {
  console.error('peer deadline');
  process.exit(1);
}, 20000).unref();
const providers = urls.map((url, index) => {
  class TicketSocket extends WebSocket {
    constructor() { super(url); }
  }
  const provider = new WebsocketProvider('ws://fixture', 'room', docs[index], {
    WebSocketPolyfill: TicketSocket,
    connect: false,
    disableBc: true
  });
  provider.awareness.setLocalState({ name: ['first', 'second', 'viewer'][index] });
  provider.on('connection-close', event => {
    if (!expectedCloses.has(index)) unexpectedCloses.push(event.code);
  });
  return provider;
});

async function until(predicate, description) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    assert.equal(unexpectedCloses.length, 0, `unexpected close: ${unexpectedCloses}`);
    assert.ok(Date.now() < deadline, description);
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}

async function run() {
  providers.forEach(provider => provider.connect());
  await until(() => providers.every(p => p.synced && p.wsconnected), 'initial synchronization');
  docs[0].getText('text').insert(0, 'α');
  docs[1].getText('text').insert(0, 'β');
  await until(() => docs.every(d => d.getText('text').length === 2)
    && new Set(docs.map(d => d.getText('text').toString())).size === 1, 'concurrent edits');
  const vector = doc => JSON.stringify([...Y.decodeStateVector(Y.encodeStateVector(doc))]
    .sort(([a], [b]) => a - b));
  assert.equal(new Set(docs.map(vector)).size, 1);
  await until(() => providers.every(p => p.awareness.getStates().size === 3), 'presence forwarding');
  expectedCloses.add(2);
  providers[2].disconnect();
  await until(() => providers.slice(0, 2).every(p => p.awareness.getStates().size === 2
    && !p.awareness.getStates().has(docs[2].clientID)), 'viewer removal');
  docs[0].getText('text').insert(0, 'γ');
  await until(() => docs.slice(0, 2).every(d => d.getText('text').length === 3)
    && docs[0].getText('text').toString() === docs[1].getText('text').toString(), 'continued editing');
  await new Promise(resolve => setTimeout(resolve, 100));
  assert.equal(unexpectedCloses.length, 0, `unexpected close: ${unexpectedCloses}`);
  assert.ok(providers.slice(0, 2).every(p => p.wsconnected));
  console.log(JSON.stringify({ converged: true, presence: true, continuedEditing: true }));
}

run().catch(error => {
  console.error(error);
  process.exitCode = 1;
}).finally(() => {
  providers.forEach((provider, index) => {
    expectedCloses.add(index);
    provider.destroy();
  });
  docs.forEach(doc => doc.destroy());
  setTimeout(() => process.exit(process.exitCode || 0), 50);
});
