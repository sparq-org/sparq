// Research grade and not externally audited.
export function canonicalArtifact(text) {
  const program = JSON.parse(text, (key, value, context) => {
    if (typeof value === 'number' && !Number.isSafeInteger(value)) return JSON.rawJSON(context.source);
    return value;
  });
  // Native cache identifiers retain their exact integer spelling. Debug sources
  // are unnecessary for execution and would embed build-machine paths.
  program.file_map = {};
  return Buffer.from(JSON.stringify(program) + '\n');
}
