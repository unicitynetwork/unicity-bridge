import type { WalletTokenPlugin } from '@unicitylabs/bridge-core';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';

import { BridgedTokenIssuancePolicy } from '../BridgedTokenIssuancePolicy.js';
import { BridgeMintJustificationVerifier } from '../BridgeMintJustificationVerifier.js';
import { LockMintJustificationVerifier } from '../LockMintJustificationVerifier.js';
import type { LoadedBridge } from './manifest.js';

export function bridgeTokenPlugin(bridge: LoadedBridge): WalletTokenPlugin {
  const { resolvedConfig, coinIdHex, verifier } = bridge.plugin;
  return {
    id: `bridge:${bridge.manifest.chainRef}:${bridge.manifest.symbol.toLowerCase()}`,
    mintJustificationVerifiers: [verifier, ...bridge.replacedVaultVerifiers],
    tokenIssuancePolicies: [new BridgedTokenIssuancePolicy(new TokenType(resolvedConfig.tokenType), [coinIdHex])],
  };
}

export function mergeBridgeTokenPlugins(plugins: readonly WalletTokenPlugin[]): WalletTokenPlugin {
  const verifiers = plugins.flatMap((p) => p.mintJustificationVerifiers);
  for (const v of verifiers) {
    if (!(v instanceof LockMintJustificationVerifier)) {
      throw new Error(`mergeBridgeTokenPlugins: verifier for tag ${v.tag} is not a bridge lock verifier`);
    }
  }
  return {
    id: 'bridge',
    mintJustificationVerifiers: [new BridgeMintJustificationVerifier(verifiers as LockMintJustificationVerifier[])],
    tokenIssuancePolicies: plugins.flatMap((p) => p.tokenIssuancePolicies),
  };
}
