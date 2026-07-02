/**
 * `ReturnServiceClient` (06 §W3) — typed client for the Part-B return service
 * (07 §B4). The wallet POSTs the {WitnessRequest} envelope and tracks the claim
 * by polling `/returns/:id`; it *also* watches the vault's `Released{nullifier}`
 * over Tron RPC independently (never trusts the service alone — 06 §A2.4). The
 * service is trustless: it can't steal or forge, only sequence + prove (07 §B2).
 */
import { toHex } from '../hex.js';
import type { WitnessRequest } from '../bridge-back/burn.js';

/** Lifecycle of a submitted return (07 §B4 — matches the service's status enum). */
export type ReturnStatus =
  | 'queued'
  | 'proving'
  | 'proven'
  | 'submitted'
  | 'settled'
  | 'failed';

/** Typed failure detail (service `failure` object). */
export interface ReturnFailure {
  readonly kind: string;
  readonly message: string;
  readonly recoverable: boolean;
}

/** A return record as the service reports it (camelCase, `/returns/:id`). */
export interface ReturnRecord {
  readonly returnId: string;
  /** 32-byte nullifier (hex) — the wallet's idempotency key. */
  readonly nullifier: string;
  readonly status: ReturnStatus;
  /** Terminal (`settled`/`failed`) — stop polling. */
  readonly terminal?: boolean;
  readonly success?: boolean | null;
  /** 0–100 coarse progress for the UI. */
  readonly progress?: number;
  /** Human status message. */
  readonly message?: string;
  /** Suggested next poll delay (ms); 0 when terminal. */
  readonly nextPollMs?: number;
  /** Batch the return landed in, once sequenced. */
  readonly batchId?: string;
  /** `fulfillBatch` txid, on settle (denormalized from the batch). */
  readonly settleTxid?: string;
  /** Typed failure detail for `failed`. */
  readonly failure?: ReturnFailure;
  /** True only on the `POST /returns` response when the nullifier was already known. */
  readonly duplicate?: boolean;
}

/** `/health` snapshot (07 §B4) — used for batch-ETA + ops display. */
export interface ReturnServiceHealth {
  readonly status: string;
  readonly queueDepth: number;
  /** The batch currently proving, if any. */
  readonly activeBatch?: string | null;
  readonly batchTarget: number;
  readonly maxWaitMs: number;
  /** `PrecheckOnly` | `Sp1Groth16`. */
  readonly proveMode: string;
}

/** The published on-chain bundle (07 §B4) — anyone can self-submit it. */
export interface BatchBundle {
  readonly batchId: string;
  readonly mode: string;
  readonly vkey?: string | null;
  readonly publicValues: string;
  readonly proofBytes: string;
  readonly settleTxid?: string | null;
}

export interface ReturnServiceClientOptions {
  /** `fetch` impl (default: the global one). Injectable for tests. */
  readonly fetch?: typeof fetch;
}

/** Thin REST client over the return service base URL. */
export class ReturnServiceClient {
  private readonly base: string;
  private readonly doFetch: typeof fetch;

  public constructor(baseUrl: string, opts: ReturnServiceClientOptions = {}) {
    this.base = baseUrl.replace(/\/+$/, '');
    // Bind to globalThis: native `fetch` requires `this` to be the real global;
    // calling it later as `this.doFetch(...)` would rebind `this` to the client
    // instance and throw "Illegal invocation".
    this.doFetch = opts.fetch ?? fetch.bind(globalThis);
  }

  /** Submit a witness request; idempotent on `nullifier`. S1 precheck rejects bad burns synchronously. */
  public async postReturn(req: WitnessRequest): Promise<ReturnRecord> {
    const body = {
      tokenCbor: toHex(req.tokenCbor),
      configHash: toHex(req.configHash),
      reasonBytes: toHex(req.reasonBytes),
      ...(req.anchorHint !== undefined ? { anchorHint: req.anchorHint.toString() } : {}),
    };
    return this.json<ReturnRecord>('POST', '/returns', body);
  }

  /** Status of a return by id. */
  public getReturn(returnId: string): Promise<ReturnRecord> {
    return this.json<ReturnRecord>('GET', `/returns/${encodeURIComponent(returnId)}`);
  }

  /** Lookup by nullifier (32-byte hex) — wallet idempotency / recovery. */
  public getByNullifier(nullifierHex: string): Promise<ReturnRecord | null> {
    return this.json<ReturnRecord | null>(
      'GET',
      `/returns?nullifier=${encodeURIComponent(nullifierHex.toLowerCase())}`,
    );
  }

  /** The published bundle for a batch (self-settle source). */
  public getBatch(batchId: string): Promise<BatchBundle> {
    return this.json<BatchBundle>('GET', `/batches/${encodeURIComponent(batchId)}`);
  }

  /** Service health (queue depth, prover busy, gas) for batch-ETA + ops UI. */
  public getHealth(): Promise<ReturnServiceHealth> {
    return this.json<ReturnServiceHealth>('GET', '/health');
  }

  private async json<T>(method: string, path: string, body?: unknown): Promise<T> {
    const res = await this.doFetch(`${this.base}${path}`, {
      method,
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (res.status === 404 && method === 'GET' && path.startsWith('/returns?')) {
      return null as T; // not-found lookup is a clean "no record", not an error.
    }
    if (!res.ok) {
      const text = await safeText(res);
      const typed = parseTypedError(text);
      if (typed) {
        throw new ReturnServiceError(typed.message, typed.code, typed.recoverable);
      }
      throw new Error(`Return service ${method} ${path} failed: HTTP ${res.status} ${text}`);
    }
    return (await res.json()) as T;
  }
}

/** `{error:{code,message,recoverable}}` (07 §B4 typed error shape). */
function parseTypedError(text: string): { code: string; message: string; recoverable: boolean } | null {
  try {
    const parsed = JSON.parse(text) as { error?: { code?: unknown; message?: unknown; recoverable?: unknown } };
    const e = parsed.error;
    if (e && typeof e.code === 'string' && typeof e.message === 'string' && typeof e.recoverable === 'boolean') {
      return { code: e.code, message: e.message, recoverable: e.recoverable };
    }
  } catch {
    /* not JSON — fall through to the generic error */
  }
  return null;
}

/** A typed return-service rejection (07 §B4). `recoverable: false` means retrying the
 * exact same request will never succeed (e.g. a stale configHash after a vault
 * redeploy) — callers should stop retrying and surface it as terminal. */
export class ReturnServiceError extends Error {
  public constructor(
    message: string,
    public readonly code: string,
    public readonly recoverable: boolean,
  ) {
    super(message);
    this.name = 'ReturnServiceError';
  }
}

async function safeText(res: Response): Promise<string> {
  try {
    return await res.text();
  } catch {
    return '';
  }
}
