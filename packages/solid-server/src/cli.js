// [GPT-5.6] sq-6xasp.9: deterministic argv/env parsing shared by the npx entry and tests.
export const DEFAULT_OPTIONS = {
  port: Number.parseInt(process.env.SPARQ_SOLID_PORT ?? process.env.PORT ?? '3000', 10),
  baseUrl: process.env.SPARQ_SOLID_BASE_URL,
  ownerWebid: process.env.SPARQ_SOLID_OWNER_WEBID,
};

export function usage() {
  return `Usage: solid-server [options]

Options:
  --port <number>       Loopback listener port (env: SPARQ_SOLID_PORT or PORT)
  --base-url <origin>   Public pod origin (env: SPARQ_SOLID_BASE_URL)
  --owner-webid <url>   Fixed local owner WebID (env: SPARQ_SOLID_OWNER_WEBID)
  --protocol <mode>     solid (default) or lws (native server)
  --native              Run Solid mode with the native server and notifications
  -h, --help            Show this help
`;
}

export function parseArgs(argv, defaults = DEFAULT_OPTIONS) {
  const options = { ...defaults };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '-h' || argument === '--help') {
      return { help: true };
    }
    if (argument === '--native') {
      options.native = true;
      continue;
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith('--')) {
      throw new Error(`${argument} requires a value`);
    }
    if (argument === '--port') {
      options.port = Number(value);
    } else if (argument === '--base-url') {
      options.baseUrl = value;
    } else if (argument === '--owner-webid') {
      options.ownerWebid = value;
    } else if (argument === '--protocol') {
      options.protocol = value;
    } else {
      throw new Error(`unknown option ${argument}`);
    }
    index += 1;
  }
  return { help: false, options };
}
