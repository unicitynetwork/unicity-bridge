function strip0x(h) {
    return h.startsWith('0x') || h.startsWith('0X') ? h.slice(2) : h;
}
/** Tron full-node HTTP API client (plain JSON — no tronweb dependency). */
export class TronHttpRpcClient {
    baseUrl;
    apiKey;
    fetchFn;
    constructor(options) {
        this.baseUrl = options.baseUrl.replace(/\/+$/, '');
        this.apiKey = options.apiKey;
        // Bind to globalThis: native `fetch` is spec'd to require `this` to be the
        // real global (window/self) — assigning the bare reference and later
        // calling it as `this.fetchFn(...)` rebinds `this` to the client instance,
        // which throws "Illegal invocation".
        const f = options.fetchFn ?? globalThis.fetch?.bind(globalThis);
        if (!f) {
            throw new Error('No fetch available; pass fetchFn in TronHttpRpcClientOptions.');
        }
        this.fetchFn = f;
    }
    async post(path, body) {
        const headers = { 'content-type': 'application/json' };
        if (this.apiKey) {
            headers['TRON-PRO-API-KEY'] = this.apiKey;
        }
        const res = await this.fetchFn(`${this.baseUrl}${path}`, {
            method: 'POST',
            headers,
            body: JSON.stringify(body),
        });
        if (!res.ok) {
            throw new Error(`Tron RPC ${path} failed: HTTP ${res.status}`);
        }
        return (await res.json());
    }
    async getTransactionInfo(txidHex) {
        const info = await this.post('/wallet/gettransactioninfobyid', { value: strip0x(txidHex) });
        if (!info || Object.keys(info).length === 0 || info.id == null) {
            return null;
        }
        const receipt = (info.receipt ?? {});
        const rawLogs = Array.isArray(info.log) ? info.log : [];
        const logs = rawLogs.map((l) => ({
            address: String(l.address ?? '').toLowerCase(),
            topics: Array.isArray(l.topics) ? l.topics.map((t) => String(t).toLowerCase()) : [],
            data: String(l.data ?? '').toLowerCase(),
        }));
        return {
            blockNumber: BigInt(info.blockNumber ?? 0),
            success: String(receipt.result ?? '') === 'SUCCESS',
            logs,
        };
    }
    async getNowBlockNumber() {
        const block = await this.post('/wallet/getnowblock', {});
        const header = (block.block_header ?? {});
        const raw = (header.raw_data ?? {});
        return BigInt(raw.number ?? 0);
    }
    /**
     * Read-only (constant) contract call — used for `allowance(owner, spender)` so
     * bridge-in can skip a redundant `approve`. Addresses go over the wire in the
     * hex `41…` form (visible:false). Throws if the node reports the call reverted
     * or returns no result word.
     */
    async triggerConstantContract(input) {
        const res = await this.post('/wallet/triggerconstantcontract', {
            owner_address: '41' + strip0x(input.ownerHex).toLowerCase(),
            contract_address: '41' + strip0x(input.contractHex).toLowerCase(),
            function_selector: input.functionSelector,
            parameter: strip0x(input.parameterHex ?? ''),
            visible: false,
        });
        const result = (res.result ?? {});
        // `result.result === true` on success; a revert sets a code/message instead.
        if (result.result !== true && (result.code != null || result.message != null)) {
            const msg = typeof result.message === 'string' ? hexMessage(result.message) : String(result.code);
            throw new Error(`triggerconstantcontract ${input.functionSelector} reverted: ${msg}`);
        }
        const cr = res.constant_result;
        if (!Array.isArray(cr) || cr.length === 0 || typeof cr[0] !== 'string') {
            throw new Error(`triggerconstantcontract ${input.functionSelector} returned no result`);
        }
        return strip0x(cr[0]);
    }
}
/** Decode a possibly-hex Tron revert message into readable text (best effort). */
function hexMessage(m) {
    if (!/^[0-9a-fA-F]*$/.test(m) || m.length % 2 !== 0)
        return m;
    try {
        const bytes = new Uint8Array(m.length / 2);
        for (let i = 0; i < bytes.length; i++)
            bytes[i] = parseInt(m.slice(i * 2, i * 2 + 2), 16);
        return new TextDecoder().decode(bytes).replace(/\0+/g, '').trim() || m;
    }
    catch {
        return m;
    }
}
