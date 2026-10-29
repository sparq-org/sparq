// [GPT-5.6] sq-6xasp.9/.10: public Node host types for the Solid-server wasm package.
// [FABLE-5] #2323: transport-agnostic dispatcher (`createSolidPod`) + host building blocks.
import type { Buffer } from 'node:buffer';
import type { IncomingMessage, Server, ServerResponse } from 'node:http';
import type { ChildProcess } from 'node:child_process';

export interface SolidServerOptions {
  port?: number;
  baseUrl?: string;
  ownerWebid?: string;
  /** Verify Solid-OIDC credentials in Node; failed or absent credentials remain anonymous. */
  oidc?: boolean;
  /** Defaults to solid. LWS always uses the optional native platform package. */
  protocol?: 'solid' | 'lws';
  /** Use the native server for Solid, including WebSocketChannel2023 notifications. */
  native?: boolean;
  /** Native mode: abort startup or stop the child with a bounded shutdown. */
  signal?: AbortSignal;
}

export interface SolidHttpServer extends Server {
  closeAsync(): Promise<void>;
}

/** A native child process with a bound loopback listener. */
export interface NativeSolidServer {
  child: ChildProcess;
  exited: Promise<{ code: number | null; signal: NodeJS.Signals | null }>;
  baseUrl: string;
  scheme: 'http' | 'https';
  address(): { address: string; family: 'IPv4'; port: number };
  closeAsync(signal?: 'SIGINT' | 'SIGTERM'): Promise<void>;
}

/** Start a loopback-only local-development server, with ephemeral storage by default. */
export function startSolidServer(options: SolidServerOptions & ({ protocol: 'lws' } | { native: true })): Promise<NativeSolidServer>;
export function startSolidServer(options?: SolidServerOptions & { protocol?: 'solid'; native?: false }): Promise<SolidHttpServer>;
export function startSolidServer(options: SolidServerOptions): Promise<SolidHttpServer | NativeSolidServer>;

export interface SolidPodOptions {
  /** Required: the public pod origin; with no listener there is no port to derive it from. */
  baseUrl: string;
  ownerWebid?: string;
  /** Verify Solid-OIDC credentials in Node; failed or absent credentials remain anonymous. */
  oidc?: boolean;
}

export interface DispatchRequest {
  /** HTTP method; defaults to GET. */
  method?: string;
  /** Origin-form request target including any query string; defaults to "/". */
  url?: string;
  /** Flat name/value pairs; repeated fields stay repeated. Defaults to none. */
  rawHeaders?: readonly string[];
  /** Pre-buffered request body; the dispatcher enforces the 2 MiB ceiling (413). */
  body?: string | Uint8Array;
}

export interface DispatchResult {
  status: number;
  /** Flat name/value pairs; repeated fields stay repeated. */
  headers: string[];
  body: Buffer;
}

export interface SolidPod {
  baseUrl: string;
  ownerWebid: string;
  /** Dispatch one request to the pod; owns body ceiling, trap recycle, and response copy. */
  dispatch(request: DispatchRequest): Promise<DispatchResult>;
  /** Release the wasm instance; dispatch() afterwards throws. */
  free(): void;
}

/** Create one in-memory wasm pod behind a transport-agnostic dispatcher (no listener). */
export function createSolidPod(options: SolidPodOptions): Promise<SolidPod>;

/** The request view handed to `authenticate` — no Node socket, just the fields it reads. */
export interface AuthenticateRequestView {
  method: string;
  url: string;
  rawHeaders: readonly string[];
  headers: Record<string, string>;
  headersDistinct: Record<string, string[]>;
}

export interface WasmPod {
  handleRequest(
    method: string,
    url: string,
    headers: readonly string[],
    body: Uint8Array,
    authenticatedWebid?: string,
  ): Promise<unknown>;
  free?(): void;
}

/** Wrap a pod factory in a trap-recovering, transport-agnostic dispatcher. */
export function createPodDispatcher(
  makePod: () => WasmPod,
  opts: { authenticate: (view: AuthenticateRequestView) => Promise<string | undefined> },
): { dispatch(request: DispatchRequest): Promise<DispatchResult>; free(): void };

/** Attach the trap-recovering dispatcher to a fresh Node `http.Server`. */
export function attachTrapRecoveryHandler(
  makePod: () => WasmPod,
  opts: { authenticate: (view: AuthenticateRequestView) => Promise<string | undefined> },
): { server: SolidHttpServer; freePod: () => void };

/** True when an error is a `WebAssembly.RuntimeError` trap that poisons the instance. */
export function isWasmTrap(error: unknown): boolean;

/** The raw wasm pod class (construct only after wasm init — `createSolidPod` handles that). */
export const SolidServer: new (baseUrl: string, ownerWebid: string) => WasmPod & {
  free(): void;
};

// http.js building blocks (byte-faithful request/response adaptation).
export const MAX_BODY_BYTES: number;
export class RequestBodyTooLargeError extends Error {}
export function flattenRequestHeaders(rawHeaders: readonly unknown[]): string[];
export function readRequestBody(request: IncomingMessage): Promise<Uint8Array>;
export function copyWasmResponse(response: unknown): DispatchResult;
export function writeNodeResponse(response: ServerResponse, result: DispatchResult): void;
