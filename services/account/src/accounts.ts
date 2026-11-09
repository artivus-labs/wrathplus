// One game account per DID, with its password kept encrypted at rest (secrets.ts).
//
//   ensure(did, handle): the DID's account and current password. Makes the account the first
//     time; afterwards only reads, so looking at your account never changes the password.
//   reset(did, handle): sets a new random password on the realm and stores it.
//
// Realm account creation is asynchronous (the worldserver queues the insert), so the commands
// that follow a create retry briefly while the realm reports the account missing.

import type { Store } from "./db.ts";
import { accountNameCandidates, type NamingOptions } from "./naming.ts";
import { generateGamePassword } from "./password.ts";
import type { PasswordCipher } from "./secrets.ts";
import { commands, SoapError, type SoapRun } from "./soap.ts";

export interface IssuedAccount {
  account: string;
  password: string;
  /** The account was made just now. */
  created: boolean;
  /** The password was set just now (new account, reset, or an unreadable stored one). */
  passwordChanged: boolean;
}

export interface AccountsDeps {
  store: Store;
  soap: SoapRun;
  cipher: PasswordCipher;
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

  ensure(did: string, handle: string): Promise<IssuedAccount> {
    return this.one(() => this.ensureNow(did, handle));
  }

  reset(did: string, handle: string): Promise<IssuedAccount> {
    return this.one(() => this.resetNow(did, handle));
  }

  private async ensureNow(did: string, handle: string): Promise<IssuedAccount> {
    const { store, cipher } = this.deps;
    const link = store.linkByDid(did);
    if (!link) return this.createNow(did, handle);
    const password = link.passwordEnc ? cipher.decrypt(did, link.passwordEnc) : undefined;
    // A link from before passwords were stored, or a lost key: the only way forward is a new one.
    if (password === undefined) return this.resetNow(did, handle);
    if (link.handle !== handle) store.touchLink(did, handle);
    // Every sign-in refreshes the realm's copy (new handle, or a module installed since).
    await this.linkInRealm(link.account, did, handle, false);
    return { account: link.account, password, created: false, passwordChanged: false };
  }

  private async resetNow(did: string, handle: string): Promise<IssuedAccount> {
    const { store, soap, cipher } = this.deps;
    const link = store.linkByDid(did);
    if (!link) return this.createNow(did, handle);
    const password = this.newPassword();
    let created = false;
    try {
      await soap(commands.setPassword(link.account, password));
    } catch (e) {
      // The realm lost the account (restored backup, manual delete): make it again, same name.
      if (!(e instanceof SoapError && e.notExists)) throw redact(e, password);
      await soap(commands.createAccount(link.account, password)).catch((err) => {
        throw redact(err, password);
      });
      created = true;
    }
    store.setPassword(did, cipher.encrypt(did, password));
    if (link.handle !== handle) store.touchLink(did, handle);
    if (created) await this.whileMissing(commands.setAddon(link.account));
    await this.linkInRealm(link.account, did, handle, created);
    return { account: link.account, password, created, passwordChanged: true };
  }

  private async createNow(did: string, handle: string): Promise<IssuedAccount> {
    const { store, soap, cipher } = this.deps;
    const password = this.newPassword();
    for (const name of accountNameCandidates(handle, did, this.deps.naming)) {
      if (store.accountTaken(name)) continue;
      try {
        await soap(commands.createAccount(name, password));
      } catch (e) {
        if (e instanceof SoapError && e.alreadyExists) continue; // not ours: never take it over
        throw redact(e, password);
      }
      store.insertLink(did, name, handle, cipher.encrypt(did, password));
      this.log(`account: created ${name} for ${did}`);
      await this.whileMissing(commands.setAddon(name));
      await this.linkInRealm(name, did, handle, true);
      return { account: name, password, created: true, passwordChanged: true };
    }
    throw new Error("no free game account name for this identity; ask the realm owner");
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

  /** Tell the realm module who this account is (welcome message, progress events). */
  private async linkInRealm(account: string, did: string, handle: string, justCreated: boolean): Promise<void> {
    try {
      const command = commands.link(account, did, handle);
      await (justCreated ? this.whileMissing(command) : this.deps.soap(command));
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
