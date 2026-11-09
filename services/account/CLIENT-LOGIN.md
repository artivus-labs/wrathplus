# Client sign-in through the browser

How the WrathPlus client and launcher get a player's game account without the player typing a
password: an OAuth-style loopback flow ([RFC 8252](https://www.rfc-editor.org/rfc/rfc8252)
loopback redirect, [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636) PKCE, S256 only). The
player signs in on `https://wrathplus.artivus.com.au` with their ATProto account (or is already
signed in there) and confirms; the client receives `{account, password, realm}`.

The service side is in `src/client.ts` and `src/app.ts`; its tests are `tests/client.test.ts`.

## Steps

1. **Listen.** Bind an HTTP listener on `127.0.0.1` (or `::1`) on a random free port. Accept
   one request to your callback path, then close it.

2. **Make the secrets.** For each sign-in, fresh:
   - `state`: 16 to 256 characters from `A-Z a-z 0-9 . _ ~ -` (32 random bytes, base64url, is
     ideal).
   - `code_verifier`: 43 to 128 characters from the same set (32 random bytes, base64url, gives
     43).
   - `code_challenge` = BASE64URL-without-padding(SHA-256(ASCII(code_verifier))): 43 characters.

3. **Open the browser** (the system browser, never an embedded web view) at

   ```
   https://wrathplus.artivus.com.au/client/login
     ?redirect=http://127.0.0.1:<port>/callback
     &state=<state>
     &code_challenge=<code_challenge>
     &code_challenge_method=S256
   ```

   URL-encode each value. `code_challenge_method` may be omitted (S256 is the only method).

   `redirect` must be exactly `http://127.0.0.1:<port>/<path>` or `http://[::1]:<port>/<path>`:
   an explicit port (any), a path of letters, digits and `. _ ~ / -`, and no query, fragment or
   user info. `localhost` is refused (it can resolve elsewhere), as is anything else. An invalid
   request shows an error page and never redirects anywhere.

4. **The player** signs in with ATProto if they have no session on the site, then sees
   "Sign in WrathPlus client as @handle?" and chooses **Sign in** or **Cancel**.

5. **Your listener receives** one of

   ```
   GET /callback?code=<code>&state=<state>&iss=https%3A%2F%2Fwrathplus.artivus.com.au
   GET /callback?error=access_denied&state=<state>&iss=https%3A%2F%2Fwrathplus.artivus.com.au
   ```

   - Compare `state` with the one you sent, exactly. If it differs, ignore the request entirely
     (do not redeem the code).
   - `iss` is always `https://wrathplus.artivus.com.au`; check it.
   - Answer the browser with a small page ("You can close this tab and return to WrathPlus").
   - The code is valid for **60 seconds** and **once**.

6. **Redeem the code:**

   ```http
   POST https://wrathplus.artivus.com.au/client/token
   Content-Type: application/json

   {"code": "<code>", "code_verifier": "<code_verifier>"}
   ```

   (`application/x-www-form-urlencoded` with the same two fields also works.)

   Success, `200`:

   ```json
   {"account": "ALICE", "password": "K7Q2…", "realm": "wrathplus.artivus.com.au:3724",
    "handle": "alice.bsky.social", "did": "did:plc:…"}
   ```

   Failure, `400`, `{"error": "invalid_request"}` (missing or malformed fields) or
   `{"error": "invalid_grant"}` (unknown, used, expired code, or wrong verifier). The first
   redemption attempt consumes the code whether or not it succeeds, so on any error start again
   from step 2. `429` (`slow_down`, with `Retry-After`) and `503` (`temporarily_unavailable`,
   the realm is unreachable) are retryable later, also from step 2.

7. **Log in** to `realm` (the realmlist host and port) with `account` and `password` as usual.

## Notes for the client

- The password returned is the player's **current** game password; this flow never changes it.
  Players reset it on the website. Keep it only as long as you need it (or in the OS keychain if
  the launcher remembers logins); never log it.
- Passwords are upper-case letters and digits; the realm ignores case.
- The flow works the same whether or not the player was already signed in on the website.
- Agents (no browser) keep using `POST /api/agent/login` with an app password; see the README.
