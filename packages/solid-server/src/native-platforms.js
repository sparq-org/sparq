// The runtime map is also consumed by the package generator and package guardrails.
export const NATIVE_PLATFORMS = Object.freeze({
  'linux-x64': { os: 'linux', cpu: 'x64', tier: 'x64-baseline' },
  'linux-arm64': { os: 'linux', cpu: 'arm64', tier: 'arm64-linux' },
  'darwin-x64': { os: 'darwin', cpu: 'x64', tier: 'x64-darwin' },
  'darwin-arm64': { os: 'darwin', cpu: 'arm64', tier: 'arm64-darwin' },
  'win32-x64': { os: 'win32', cpu: 'x64', tier: 'win-x64-baseline' },
});

export function nativePackageName(platform = process.platform, arch = process.arch) {
  const key = `${platform}-${arch}`;
  if (!Object.hasOwn(NATIVE_PLATFORMS, key)) {
    throw new Error(`native server is unavailable for ${key}; supported platforms: ${Object.keys(NATIVE_PLATFORMS).join(', ')}`);
  }
  return `@sparq-org/solid-server-native-${key}`;
}
