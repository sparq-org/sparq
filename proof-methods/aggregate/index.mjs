import { createRequire } from 'node:module';

const aggregate = createRequire(import.meta.url)('./aggregate.js');

export const {
  newSession,
  generateKeys,
  contribute,
  receiveAndPartial,
  finish,
  verifyOutcome,
  simulateSession,
} = aggregate;
