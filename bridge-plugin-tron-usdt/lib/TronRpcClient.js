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
}
