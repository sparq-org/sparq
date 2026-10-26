#!/usr/bin/env node
// [GPT-5.6] sq-6xasp.9: npx entry for the local fixed-owner wasm Solid server.
import { startSolidServer } from '../src/index.js';
import { parseArgs, usage } from '../src/cli.js';
import { constants } from 'node:os';

async function main() {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed.help) {
    process.stdout.write(usage());
    return;
  }

  let server;
  let pendingSignal;
  const shutdown = (signal) => {
    pendingSignal = signal;
    if (!server) return;
    if (server.child) {
      server.child.kill(signal);
    } else {
      void server.closeAsync().catch((error) => {
        process.stderr.write(`${error.message}\n`);
        process.exitCode = 1;
      });
    }
  };
  const onInt = () => shutdown('SIGINT');
  const onTerm = () => shutdown('SIGTERM');
  process.once('SIGINT', onInt);
  process.once('SIGTERM', onTerm);
  try {
    server = await startSolidServer(parsed.options);
  } catch (error) {
    process.off('SIGINT', onInt);
    process.off('SIGTERM', onTerm);
    throw error;
  }
  if (pendingSignal) shutdown(pendingSignal);
  const address = server.address();
  if (!address || typeof address === 'string') {
    throw new Error('listener did not expose a TCP address');
  }
  process.stdout.write(`${JSON.stringify({
    event: 'listening',
    url: `http://127.0.0.1:${address.port}`,
    baseUrl: parsed.options.baseUrl ?? `http://127.0.0.1:${address.port}`,
    ...(server.child ? {
      protocol: parsed.options.protocol ?? 'solid',
      auth: 'native',
    } : {
      ownerWebid: parsed.options.ownerWebid ?? 'https://example.invalid/profile/card#me',
      auth: 'fixed-owner-local-only',
    }),
  })}\n`);

  if (server.child) {
    const { code, signal } = await server.exited;
    process.off('SIGINT', onInt);
    process.off('SIGTERM', onTerm);
    process.exitCode = code ?? (128 + (constants.signals[signal] ?? 1));
  }
}

main().catch((error) => {
  process.stderr.write(`solid-server: ${error.message}\n`);
  process.exitCode = error.nativeExitCode ?? (error.nativeSignal ? 128 + (constants.signals[error.nativeSignal] ?? 1) : 1);
});
