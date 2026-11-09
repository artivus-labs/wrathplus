// One game account per DID. After an identity is proven, `issue` makes the DID's account (first
// time) or finds it (afterwards), sets a fresh random password, and returns it. The password is
// never stored: the caller shows it once. Every sign-in therefore issues a new password.
//
// Realm account creation is asynchronous (the worldserver queues the insert), so the commands
// that follow it retry briefly while the realm reports the account missing.

import type { Store } from "./db.ts";
import { accountNameCandidates, type NamingOptions } from "./naming.ts";
import { generateGamePassword } from "./password.ts";
import { commands, SoapError, type SoapRun } from "./soap.ts";

export interface IssuedAccount {
  account: string;
  password: string;
  created: boolean;
}

export interface AccountsDeps {
  store: Store;
  soap: SoapRun;
  naming?: NamingOptions;
  password?: () => string;
  wait?: (ms: number) => Promise<void>;
  log?: (message: string) => void;
}

/** A queue that runs one task at a time (account allocation must not race itself). */
function serial() {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(task: () => Promise<T>): Promise<T> => {
    const run = tail.then(task, task);
    tail = run.catch(() => undefined);
    return run;
  };
}

export class Accounts {
  private readonly one = serial();
  private readonly wait: (ms: number) => Promise<void>;
  private readonly log: (message: string) => void;
  private readonly newPassword: () => string;

  constructor(private readonly deps: AccountsDeps) {
    this.wait = deps.wait ?? ((ms) => new Promise((r) => setTimeout(r, ms)));
    this.log = deps.log ?? ((m) => console.log(m));
    this.newPassword = deps.password ?? (() => generateGamePassword());
  }

  issue(did: string, handle: string): Promise<IssuedAccount> {
    return this.one(() => this.issueNow(did, handle));
  }

  /** Retry `command` while the realm says the account does not exist yet. */
  private async whileMissing(command: string): Promise<string> {
    for (let attempt = 1; ; attempt++) {
      try {
        return await this.deps.soap(command);
      } catch (e) {
        if (!(e instanceof SoapError && e.notExists) || attempt >= 20) throw e;
        await this.wait(250);
      }
    }
  }

  private async issueNow(did: string, handle: string): Promise<IssuedAccount> {
    const { store, soap } = this.deps;
    const password = this.newPassword();
    const existing = store.linkByDid(did);
    let account: string;
    let created = false;
    if (existing) {
      account = existing.account;
      try {
        await soap(commands.setPassword(account, password));
      } catch (e) {
        // The realm lost the account (restored backup, manual delete): make it again, same name.
        if (!(e instanceof SoapError && e.notExists)) throw redact(e, password);
        await soap(commands.createAccount(account, password)).catch((err) => {
          throw redact(err, password);
        });
        created = true;
      }
      store.touchLink(did, handle);
    } else {
      account = await this.createFresh(did, handle, password);
      created = true;
    }
    if (created) await this.whileMissing(commands.setAddon(account));
    await this.linkInRealm(account, did, handle);
    return { account, password, created };
  }

  private async createFresh(did: string, handle: string, password: string): Promise<string> {
    const { store, soap } = this.deps;
    for (const name of accountNameCandidates(handle, did, this.deps.naming)) {
      if (store.accountTaken(name)) continue;
      try {
        await soap(commands.createAccount(name, password));
      } catch (e) {
        if (e instanceof SoapError && e.alreadyExists) continue; // not ours: never take it over
        throw redact(e, password);
      }
      store.insertLink(did, name, handle);
      this.log(`account: created ${name} for ${did}`);
      return name;
    }
    throw new Error("no free game account name for this identity; ask the realm owner");
  }

  /** Tell the realm module who this account is (welcome message, progress events). */
  private async linkInRealm(account: string, did: string, handle: string): Promise<void> {
    try {
      await this.whileMissing(commands.link(account, did, handle));
    } catch (e) {
      // The account works without the module; only the in-game handle and records are missing.
      this.log(`account: realm link for ${account} failed: ${(e as Error).message}`);
    }
  }
}

function redact(e: unknown, password: string): Error {
  const message = e instanceof Error ? e.message : String(e);
  const clean = message.split(password).join("…");
  return e instanceof SoapError ? new SoapError(clean, e.status) : new Error(clean);
}
