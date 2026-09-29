import type { WalletTokenPlugin } from '@unicitylabs/bridge-core';
import { TokenType } from '@unicitylabs/state-transition-sdk/lib/transaction/TokenType.js';
import type { IMintJustificationVerifier } from '@unicitylabs/state-transition-sdk/lib/transaction/verification/IMintJustificationVerifier.js';

import { BridgedTokenIssuancePolicy } from '../BridgedTokenIssuancePolicy.js';
import { BridgeMintJustificationVerifier } from '../BridgeMintJustificationVerifier.js';
import { LockMintJustificationVerifier } from '../LockMintJustificationVerifier.js';
import type { LoadedBridge } from './manifest.js';

export function bridgeTokenPlugin(bridge: LoadedBridge): WalletTokenPlugin {
  const { resolvedConfig, coinIdHex, verifier } = bridge.plugin;
  const lockVerifiers = [verifier, ...bridge.replacedVaultVerifiers];
  const revision = lockVerifiers.map((v) => v.trustAnchor).sort().join(',');
  return {
    id: `bridge:${bridge.manifest.chainRef}:${bridge.manifest.symbol.toLowerCase()}`,
    mintJustificationVerifiers: [new BridgeMintJustificationVerifier(lockVerifiers)],
    tokenIssuancePolicies: [new BridgedTokenIssuancePolicy(new TokenType(resolvedConfig.tokenType), [coinIdHex], revision)],
  };
}

export function mergeBridgeTokenPlugins(plugins: readonly WalletTokenPlugin[]): WalletTokenPlugin {
  return {
    id: 'bridge',
    mintJustificationVerifiers: [
      new BridgeMintJustificationVerifier(plugins.flatMap((p) => p.mintJustificationVerifiers).flatMap(lockVerifiersOf)),
    ],
    tokenIssuancePolicies: plugins.flatMap((p) => p.tokenIssuancePolicies),
  };
}

function lockVerifiersOf(verifier: IMintJustificationVerifier): readonly LockMintJustificationVerifier[] {
  if (verifier instanceof BridgeMintJustificationVerifier) return verifier.lockVerifiers;
  if (verifier instanceof LockMintJustificationVerifier) return [verifier];
  throw new Error(`mergeBridgeTokenPlugins: verifier for tag ${verifier.tag} is not a bridge lock verifier`);
}
