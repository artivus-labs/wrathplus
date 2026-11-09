// Game passwords. AzerothCore takes at most 16 characters (MAX_PASS_STR) and upper-cases the
// password before hashing it (SRP6), so only upper-case letters and digits carry entropy. The
// alphabet drops characters that look alike (0/O, 1/I): 32 symbols x 16 = 80 bits, drawn
// without modulo bias (32 divides 256).

const ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
export const GAME_PASSWORD_LENGTH = 16;
export const GAME_PASSWORD = /^[A-Z0-9]{16}$/;

export function generateGamePassword(random: (bytes: Uint8Array) => Uint8Array = (b) => crypto.getRandomValues(b)): string {
  const bytes = random(new Uint8Array(GAME_PASSWORD_LENGTH));
  let out = "";
  for (const byte of bytes) out += ALPHABET[byte & 31];
  return out;
}
