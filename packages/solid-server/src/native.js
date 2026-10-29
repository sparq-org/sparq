import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { nativePackageName } from './native-platforms.js';

const require = createRequire(import.meta.url);
const host = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));

export function resolveNativeBinary() {
  const name = nativePackageName();
  const executable = process.platform === 'win32' ? 'sparq-lws-core.exe' : 'sparq-lws-core';
  let manifest;
  let binary;
  try {
    manifest = JSON.parse(readFileSync(require.resolve(`${name}/package.json`), 'utf8'));
    binary = require.resolve(`${name}/bin/${executable}`);
  } catch (cause) {
    throw new Error(`missing native server package ${name}; install ${name}@${host.version} and enable optional dependencies`, { cause });
  }
  if (manifest.name !== name || manifest.version !== host.version) {
    throw new Error(`native server package ${name} must match @sparq-org/solid-server version ${host.version}`);
  }
  return binary;
}

export async function startNativeServer(options, { normalizePort, normalizeBaseUrl }) {
  if (options.ownerWebid !== undefined || options.oidc !== undefined) {
    throw new TypeError('native mode uses SOLID_SERVER_* authentication settings; ownerWebid and oidc are wasm host options');
  }
  if (options.signal !== undefined && !(options.signal instanceof AbortSignal)) {
    throw new TypeError('signal must be an AbortSignal');
  }
  options.signal?.throwIfAborted();
  const requestedPort = normalizePort(options.port);
  const baseUrl = normalizeBaseUrl(options.baseUrl, requestedPort);
  const binary = resolveNativeBinary();
  const env = {
    ...process.env,
    SOLID_SERVER_PROTOCOL: options.protocol ?? 'solid',
    SOLID_SERVER_BIND: `127.0.0.1:${requestedPort}`,
    SOLID_SERVER_BASE_URL: baseUrl,
    SOLID_SERVER_READY_STDOUT: '1',
    PSS_SPARQ_BACKEND: process.env.PSS_SPARQ_BACKEND ?? 'memory',
  };
  const child = spawn(binary, [], { env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
  let stderr = '';
  child.stderr.on('data', (chunk) => {
    stderr = (stderr + chunk).slice(-8192);
  });
  child.stderr.pipe(process.stderr, { end: false });
  const exited = new Promise((resolve) => child.once('close', (code, signal) => resolve({ code, signal })));
  let stopping;
  const stop = (signal = 'SIGTERM') => {
    if (!stopping) {
      stopping = (async () => {
        if (child.exitCode === null && child.signalCode === null) child.kill(signal);
        const timer = setTimeout(() => child.kill('SIGKILL'), 10_000);
        timer.unref();
        try { await exited; } finally { clearTimeout(timer); }
      })();
    }
    return stopping;
  };
  const onAbort = () => { void stop(options.signal.reason === 'SIGINT' ? 'SIGINT' : 'SIGTERM'); };
  options.signal?.addEventListener('abort', onAbort, { once: true });
  void exited.then(() => options.signal?.removeEventListener('abort', onAbort));
  try {
    const address = await new Promise((resolve, reject) => {
      let pending = '';
      let settled = false;
      const finish = (error, address) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        if (error) reject(error); else resolve(address);
      };
      const timer = setTimeout(() => finish(new Error('native server did not report its bound listener')), 30_000);
      child.once('error', (error) => finish(error));
      void exited.then(({ code, signal }) => finish(Object.assign(
        new Error(`native server exited during startup: ${stderr.trim()}`),
        { nativeExitCode: code, nativeSignal: signal },
      )));
      child.stdout.setEncoding('utf8');
      child.stdout.on('data', (chunk) => {
        if (settled) return;
        // The child reports its bound listener in one bounded JSON line.
        if (pending.length + chunk.length > 8192) {
          finish(new Error('native server listener report is too large'));
          return;
        }
        pending += chunk;
        if (!pending.includes('\n')) return;
        try {
          const report = JSON.parse(pending.slice(0, pending.indexOf('\n')));
          const match = /^127\.0\.0\.1:(\d+)$/.exec(report.address);
          const port = Number(match?.[1]);
          if (report.event !== 'native-listening' || !['http', 'https'].includes(report.scheme)
            || !Number.isInteger(port) || port < 1 || port > 65535
            || (requestedPort !== 0 && port !== requestedPort)) {
            throw new Error('native server returned an invalid listener report');
          }
          finish(null, { address: '127.0.0.1', family: 'IPv4', port, scheme: report.scheme });
        } catch (error) { finish(error); }
        pending = '';
      });
    });
    options.signal?.throwIfAborted();
    return {
      child, exited, baseUrl, scheme: address.scheme,
      address: () => ({ address: address.address, family: address.family, port: address.port }),
      closeAsync: (signal) => stop(signal),
    };
  } catch (error) {
    await stop();
    throw error;
  }
}
