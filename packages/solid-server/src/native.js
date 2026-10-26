import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { createConnection, createServer } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { nativePackageName } from './native-platforms.js';

const require = createRequire(import.meta.url);

export function resolveNativeBinary() {
  const name = nativePackageName();
  const executable = process.platform === 'win32' ? 'sparq-lws-core.exe' : 'sparq-lws-core';
  try {
    return require.resolve(`${name}/bin/${executable}`);
  } catch (cause) {
    throw new Error(`missing native server package ${name}; install ${name} at the same version as @sparq-org/solid-server and enable optional dependencies`, { cause });
  }
}

async function choosePort(port) {
  const probe = createServer();
  await new Promise((resolve, reject) => {
    probe.once('error', reject);
    probe.listen(port, '127.0.0.1', resolve);
  });
  const chosen = probe.address().port;
  await new Promise((resolve, reject) => probe.close((error) => error ? reject(error) : resolve()));
  return chosen;
}

function listening(port) {
  return new Promise((resolve) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    const finish = (ready) => { socket.destroy(); resolve(ready); };
    socket.once('connect', () => finish(true));
    socket.once('error', () => finish(false));
    socket.setTimeout(200, () => finish(false));
  });
}

export async function startNativeServer(options, { normalizePort, normalizeBaseUrl }) {
  if (options.ownerWebid !== undefined || options.oidc !== undefined) {
    throw new TypeError('native mode uses SOLID_SERVER_* authentication settings; ownerWebid and oidc are wasm host options');
  }
  if (options.data !== undefined && (typeof options.data !== 'string' || !options.data.trim())) {
    throw new TypeError('data must be a non-empty directory path');
  }
  const requestedPort = normalizePort(options.port);
  const baseUrl = normalizeBaseUrl(options.baseUrl, requestedPort);
  const binary = resolveNativeBinary();
  const port = await choosePort(requestedPort);
  const env = {
    ...process.env,
    SOLID_SERVER_PROTOCOL: options.protocol ?? 'solid',
    SOLID_SERVER_BIND: `127.0.0.1:${port}`,
    SOLID_SERVER_BASE_URL: baseUrl,
    PSS_SPARQ_BACKEND: process.env.PSS_SPARQ_BACKEND ?? 'memory',
  };
  if (options.data !== undefined) {
    env.PSS_SPARQ_BACKEND = 'embedded';
    env.SOLID_SERVER_SPARQ_DIR = options.data;
  }
  const child = spawn(binary, [], { env, stdio: ['ignore', 'inherit', 'pipe'], windowsHide: true });
  let stderr = '';
  let spawnError;
  child.stderr.on('data', (chunk) => {
    stderr = (stderr + chunk).slice(-8192);
    process.stderr.write(chunk);
  });
  child.once('error', (error) => { spawnError = error; });
  const exited = new Promise((resolve) => child.once('close', (code, signal) => resolve({ code, signal })));
  const stop = async (signal = 'SIGTERM') => {
    if (child.exitCode === null && child.signalCode === null) child.kill(signal);
    const timer = setTimeout(() => child.kill('SIGKILL'), 10_000);
    timer.unref();
    try { await exited; } finally { clearTimeout(timer); }
  };
  try {
    const deadline = Date.now() + 30_000;
    for (;;) {
      if (spawnError) throw spawnError;
      if (child.exitCode !== null || child.signalCode !== null) {
        throw Object.assign(new Error(`native server exited during startup: ${stderr.trim()}`), {
          nativeExitCode: child.exitCode, nativeSignal: child.signalCode,
        });
      }
      if (await listening(port)) break;
      if (Date.now() >= deadline) throw new Error('native server did not start listening');
      await delay(25);
    }
  } catch (error) {
    await stop();
    throw error;
  }
  return {
    child, exited, baseUrl,
    address: () => ({ address: '127.0.0.1', family: 'IPv4', port }),
    closeAsync: () => stop(),
  };
}
