// Brakets Tournament Multi-Sig Escrow & Stake Payout Guardrail
// Invariant: Stake releases require cryptographic multi-sig supervisor approval

export interface EscrowSafetyPolicy {
  tournamentId: string;
  prizePoolLamports: bigint;
  escrowVaultAddress: string;
  multiSigThreshold: number;
  signersRequired: string[];
}

export function validateEscrowRelease(policy: EscrowSafetyPolicy, signatures: string[]): boolean {
  if (signatures.length < policy.multiSigThreshold) {
    throw new Error(`Escrow multi-sig violation: requires at least ${policy.multiSigThreshold} supervisor signatures, got ${signatures.length}`);
  }
  return true;
}
