// [GPT-5.6] #1046 — the binary-to-RDF boundary shared by file uploads and URL imports.
// Read compressed sources as bytes, decode their container before UTF-8, then select the
// RDF syntax from the inner name. Keeping this framework-free makes both invocation paths
// unit-testable without mounting the former /try workbench.
//
// #5114 — decoding is `@sparq/client`'s `decompressDatasetBytes` (the one browser
// decompressor the GUI also uses); this module only applies the site's import policy.

import {
  datasetCodecFromContentType,
  datasetCodecFromName,
  decompressDatasetBytes,
  sniffDatasetCodec,
  type DatasetCompressionCodec,
} from "@sparq/client";
import { formatFromContentType, guessFormat } from "./repl-dataset";

/** The archive codecs the site imports. bzip2 is recognised but deliberately not offered. */
export type ArchiveCodec = Exclude<DatasetCompressionCodec, "bzip2">;

/** Stable error used when bytes do not name one of the site's import codecs. */
export const UNSUPPORTED_ARCHIVE_ERROR =
  "Unrecognised compressed payload: expected a gzip (.gz), zip (.zip), or zstd (.zst) magic number.";

const siteCodec = (
  codec: DatasetCompressionCodec | undefined,
): ArchiveCodec | undefined => (codec === "bzip2" ? undefined : codec);

export interface BytesToRdfOptions {
  /** An explicit format picker value; `__auto__` keeps automatic detection enabled. */
  explicitFormat?: string;
  /** The response media type for URL imports, including an optional charset parameter. */
  contentType?: string | null;
}

export interface RdfSource {
  /** UTF-8 RDF source text, decoded only after any archive container is removed. */
  text: string;
  /** The engine format inferred from the inner RDF name/media type or explicitly selected. */
  format: string;
}

/**
 * Turns uploaded/fetched bytes into RDF source text and its engine format. Archive detection
 * follows the existing URL contract (container Content-Type, source suffix, then magic bytes),
 * while plain RDF prefers its served media type over the source suffix. gzip/zip stay on the
 * browser-native archive path; zstd lazy-loads its decoder inside `@sparq/client`.
 */
export async function bytesToRdf(
  bytes: Uint8Array,
  sourceName: string,
  options: BytesToRdfOptions = {},
): Promise<RdfSource> {
  const { explicitFormat, contentType } = options;
  const selectedFormat =
    explicitFormat && explicitFormat !== "__auto__"
      ? explicitFormat
      : undefined;
  const signals = [
    datasetCodecFromContentType(contentType),
    datasetCodecFromName(sourceName),
    sniffDatasetCodec(bytes),
  ];
  const archiveCodec = signals.map(siteCodec).find((codec) => codec);

  // bzip2 is deliberately not a site import codec. Reject its established signals so a URL
  // response cannot fall through to UTF-8 and reach the RDF parser as corrupted text.
  if (!archiveCodec && signals.includes("bzip2")) {
    throw new Error(UNSUPPORTED_ARCHIVE_ERROR);
  }

  if (archiveCodec) {
    const { bytes: decoded, innerName } = await decompressDatasetBytes(
      bytes,
      sourceName,
      archiveCodec,
    );
    return {
      text: new TextDecoder().decode(decoded),
      format: selectedFormat ?? guessFormat(innerName ?? sourceName),
    };
  }

  return {
    text: new TextDecoder().decode(bytes),
    format:
      selectedFormat ??
      formatFromContentType(contentType ?? null) ??
      guessFormat(sourceName),
  };
}

/** The site import codec a picked file announces (magic first, then name), or `undefined`. */
export function archiveCodecForFile(
  bytes: Uint8Array,
  name: string,
): ArchiveCodec | undefined {
  return siteCodec(sniffDatasetCodec(bytes)) ?? siteCodec(datasetCodecFromName(name));
}
