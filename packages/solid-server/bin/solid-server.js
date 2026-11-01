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
  const native = parsed.options.native || parsed.options.protocol === 'lws';
  const controller = new AbortController();
  const close = (signal) => {
    if (server.child) {
      void server.closeAsync(signal);
    } else {
      void server.closeAsync().catch((error) => {
        process.stderr.write(`${error.message}\n`);
        process.exitCode = 1;
      });
    }
  };
  const shutdown = (signal) => {
    if (!native && pendingSignal) {
      process.exit(128 + constants.signals[signal]);
    }
    pendingSignal = signal;
    controller.abort(signal);
    if (server) close(signal);
  };
  const onInt = () => shutdown('SIGINT');
  const onTerm = () => shutdown('SIGTERM');
  process.on('SIGINT', onInt);
  process.on('SIGTERM', onTerm);
  try {
    server = await startSolidServer({
      ...parsed.options,
      ...(native ? { signal: controller.signal } : {}),
    });
  } catch (error) {
    process.off('SIGINT', onInt);
    process.off('SIGTERM', onTerm);
    if (controller.signal.aborted && error === controller.signal.reason) {
      throw Object.assign(new Error(`startup interrupted by ${pendingSignal}`), { nativeSignal: pendingSignal });
    }
    throw error;
  }
  if (pendingSignal) close(pendingSignal);
  const address = server.address();
  if (!address || typeof address === 'string') {
    throw new Error('listener did not expose a TCP address');
  }
  process.stdout.write(`${JSON.stringify({
    event: 'listening',
    url: `${server.scheme ?? 'http'}://127.0.0.1:${address.port}`,
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
