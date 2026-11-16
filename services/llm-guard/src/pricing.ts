// What one Messages API response costs, from its `usage` block.

/** Token counts as the Messages API reports them (absent fields count as zero). */
export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
  /** Breakdown of cache writes by TTL, when the API sends it. */
  cache_creation?: { ephemeral_5m_input_tokens?: number; ephemeral_1h_input_tokens?: number };
}

/** USD per million tokens. */
export interface Price {
  input: number;
  output: number;
  /** Rates for a request whose prompt is longer than `longAbove` tokens (the whole request). */
  longInput: number;
  longOutput: number;
  longAbove: number;
}

/** Cache writes (5-minute TTL) and reads, as multiples of the input rate. */
export const CACHE_WRITE = 1.25;
export const CACHE_WRITE_1H = 2;
export const CACHE_READ = 0.1;

/** Models the guard knows a price for. A model missing here cannot be allowed. */
export const PRICES: Record<string, Price> = {
  "claude-haiku-5-5": { input: 0.1, output: 0.5, longInput: 0.5, longOutput: 2.5, longAbove: 100_000 },
};

const n = (value: number | undefined) => (typeof value === "number" && Number.isFinite(value) && value > 0 ? value : 0);

/** Every prompt token: uncached input, cache writes and cache reads. */
export function promptTokens(usage: Usage): number {
  return n(usage.input_tokens) + n(usage.cache_creation_input_tokens) + n(usage.cache_read_input_tokens);
}

export function costUsd(usage: Usage, price: Price): number {
  const long = promptTokens(usage) > price.longAbove;
  const input = long ? price.longInput : price.input;
  const output = long ? price.longOutput : price.output;
  const writes = n(usage.cache_creation_input_tokens);
  // 1-hour cache writes cost more; without the breakdown every write is priced as 5-minute.
  const writes1h = Math.min(writes, n(usage.cache_creation?.ephemeral_1h_input_tokens));
  const micro =
    n(usage.input_tokens) * input +
    (writes - writes1h) * input * CACHE_WRITE +
    writes1h * input * CACHE_WRITE_1H +
    n(usage.cache_read_input_tokens) * input * CACHE_READ +
    // output_tokens already includes thinking tokens.
    n(usage.output_tokens) * output;
  return micro / 1_000_000;
}
