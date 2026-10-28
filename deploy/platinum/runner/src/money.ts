// Money is integer micro-USD (1 USD = 1_000_000) everywhere inside the runner. Floats exist only at the env/JSON edge.

export const MICRO = 1_000_000;
export const usdToMicro = (usd: number): number => Math.round(usd * MICRO);
export const microToUsd = (micro: number): number => micro / MICRO;
/** Stripe minor units (cents) -> micro-USD. */
export const centsToMicro = (cents: number): number => Math.round(cents) * 10_000;
export const microToCents = (micro: number): number => Math.round(micro / 10_000);
export const fmtUsd = (micro: number): string => `$${(micro / MICRO).toFixed(micro % 10_000 === 0 ? 2 : 6)}`;
