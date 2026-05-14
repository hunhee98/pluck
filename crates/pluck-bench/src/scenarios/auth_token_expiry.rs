//! Fixture: a 92-file TS repo with a seeded auth-token-expiry bug.
//!
//! The bug: `src/auth/session.ts` checks `s.expiresAt > now()` to detect
//! an EXPIRED session — inverted. Correct comparison is `<`. An agent
//! that walks the file must surface this line; the verifier asserts the
//! exact substring `s.expiresAt > now()` shows up in tool outputs.

use super::Scenario;

pub fn scenario() -> Scenario {
    Scenario {
        name: "fix-auth-token-expiry",
        task_prompt:
            "Find and fix the auth-session expiry check. Sessions should be \
             treated as EXPIRED when expiresAt is in the past (< now()), but \
             the current code inverts the comparison.",
        repo: build_repo(),
        bug_marker: "s.expiresAt > now()",
        bug_file: "src/auth/session.ts",
        bug_line: 35,
    }
}

fn build_repo() -> Vec<(String, String)> {
    let mut r: Vec<(String, String)> = vec![
        ("src/auth/session.ts".into(), session_ts_with_bug()),
        ("src/auth/login.ts".into(), login_ts()),
        ("src/auth/password.ts".into(), password_ts()),
        ("src/payment/charge.ts".into(), charge_ts()),
        ("src/payment/refund.ts".into(), refund_ts()),
        ("src/payment/subscription.ts".into(), subscription_ts()),
        ("src/user/profile.ts".into(), profile_ts()),
        ("src/user/settings.ts".into(), settings_ts()),
        ("src/routes/api.ts".into(), routes_ts()),
        ("src/middleware/auth.ts".into(), middleware_ts()),
        ("src/db/pool.ts".into(), db_ts()),
        ("src/utils/time.ts".into(), utils_ts()),
    ];
    for i in 0..80 {
        let path = format!("src/noise/mod_{i}.ts");
        r.push((path, noise_module(i)));
    }
    r
}

// ── Subject-matter files ────────────────────────────────────────────────────

/// session.ts — SEEDED BUG on the expiry comparison line.
/// `s.expiresAt > now()` should be `<`. Currently sessions are flagged as
/// expired only AFTER they would still be valid — the opposite of intent.
fn session_ts_with_bug() -> String {
    String::from(
        r#"import { Database } from "../db/pool";
import { now } from "../utils/time";

export interface Session {
  id: string;
  userId: string;
  createdAt: number;
  expiresAt: number;
  refreshToken: string;
}

const SESSION_TTL_MS = 1000 * 60 * 60 * 24;
const REFRESH_TTL_MS = 1000 * 60 * 60 * 24 * 30;

export async function createSession(db: Database, userId: string): Promise<Session> {
  const id = crypto.randomUUID();
  const refreshToken = "rt_" + crypto.randomUUID().replaceAll("-", "");
  const session: Session = {
    id, userId,
    createdAt: now(),
    expiresAt: now() + SESSION_TTL_MS,
    refreshToken,
  };
  await db.sessions.insert(session);
  return session;
}

/// Returns true when the session has expired and should be rejected.
///
/// NOTE: the comparison below is currently inverted — this is the seeded
/// bug for the fix-auth-token-expiry benchmark scenario. Real code must
/// use `<`, not `>`.
export async function isSessionExpired(s: Session): Promise<boolean> {
  if (!s) return true;
  return s.expiresAt > now();
}

export async function refreshSession(db: Database, refreshToken: string): Promise<Session | null> {
  const session = await db.sessions.findByRefreshToken(refreshToken);
  if (!session) return null;
  if (session.createdAt + REFRESH_TTL_MS < now()) {
    await db.sessions.delete(session.id);
    return null;
  }
  session.expiresAt = now() + SESSION_TTL_MS;
  await db.sessions.update(session);
  return session;
}

export async function revokeSession(db: Database, id: string): Promise<void> {
  await db.sessions.delete(id);
}
"#,
    )
}

fn login_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";
import { verifyPassword } from "./password";
import { createSession, Session } from "./session";

export type LoginRequest = { email: string; password: string };

export async function handleLogin(db: Database, req: LoginRequest): Promise<Session | null> {
  const user = await db.users.findByEmail(req.email);
  if (!user) return null;
  const ok = await verifyPassword(req.password, user.passwordHash);
  if (!ok) return null;
  return createSession(db, user.id);
}

export async function handleLogout(db: Database, sessionId: string): Promise<void> {
  await db.sessions.delete(sessionId);
}
"#,
    )
}

fn password_ts() -> String {
    String::from(
        r#"import { scrypt, timingSafeEqual } from "crypto";

export async function hashPassword(password: string, salt: Buffer): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    scrypt(password, salt, 64, (err, derived) => {
      if (err) reject(err);
      else resolve(derived);
    });
  });
}

export async function verifyPassword(password: string, storedHash: string): Promise<boolean> {
  const [saltHex, hashHex] = storedHash.split(":");
  const salt = Buffer.from(saltHex, "hex");
  const expected = Buffer.from(hashHex, "hex");
  const actual = await hashPassword(password, salt);
  return actual.length === expected.length && timingSafeEqual(actual, expected);
}
"#,
    )
}

fn charge_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

export type ChargeRequest = { userId: string; amountCents: number; currency: "USD" | "KRW" };

export async function chargeCustomer(db: Database, req: ChargeRequest): Promise<{ ok: boolean }> {
  const card = await db.cards.primaryFor(req.userId);
  if (!card) return { ok: false };
  await db.charges.insert({ id: "ch_" + crypto.randomUUID(), ...req, createdAt: Date.now() });
  return { ok: true };
}
"#,
    )
}

fn refund_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

const REFUND_WINDOW_MS = 1000 * 60 * 60 * 24 * 30;

export async function processRefund(db: Database, chargeId: string, amountCents: number): Promise<{ ok: boolean }> {
  const charge = await db.charges.findById(chargeId);
  if (!charge) return { ok: false };
  if (Date.now() - charge.createdAt > REFUND_WINDOW_MS) return { ok: false };
  await db.refunds.insert({ id: "rf_" + crypto.randomUUID(), chargeId, amountCents, createdAt: Date.now() });
  return { ok: true };
}
"#,
    )
}

fn subscription_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";
import { chargeCustomer } from "./charge";

export async function billPendingSubscriptions(db: Database): Promise<number> {
  const due = await db.subscriptions.findDue();
  let billed = 0;
  for (const sub of due) {
    const r = await chargeCustomer(db, { userId: sub.userId, amountCents: 1000, currency: "USD" });
    if (r.ok) billed++;
  }
  return billed;
}
"#,
    )
}

fn profile_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

export async function getProfile(db: Database, userId: string): Promise<unknown | null> {
  return db.profiles.findByUser(userId);
}

export async function updateProfile(db: Database, userId: string, patch: any): Promise<unknown> {
  return db.profiles.upsert({ ...patch, userId });
}
"#,
    )
}

fn settings_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

export async function getSettings(db: Database, userId: string): Promise<unknown> {
  return db.settings.findByUser(userId);
}
"#,
    )
}

fn routes_ts() -> String {
    String::from(
        r#"import { handleLogin, handleLogout } from "../auth/login";
import { isSessionExpired } from "../auth/session";

export async function dispatch(method: string, path: string, body: any, ctx: any) {
  if (method === "POST" && path === "/auth/login") return handleLogin(ctx.db, body);
  if (method === "POST" && path === "/auth/logout") return handleLogout(ctx.db, body.sessionId);
  return { status: 404 };
}
"#,
    )
}

fn middleware_ts() -> String {
    String::from(
        r#"import { isSessionExpired } from "../auth/session";
import { Database } from "../db/pool";

export async function requireAuth(db: Database, sessionId: string): Promise<{ ok: boolean; userId?: string }> {
  const session = await db.sessions.findById(sessionId);
  if (!session) return { ok: false };
  if (await isSessionExpired(session)) return { ok: false };
  return { ok: true, userId: session.userId };
}
"#,
    )
}

fn db_ts() -> String {
    String::from(
        r#"export interface Database {
  users: any;
  sessions: any;
  charges: any;
  refunds: any;
  subscriptions: any;
  profiles: any;
  settings: any;
  cards: any;
}
"#,
    )
}

fn utils_ts() -> String {
    String::from(
        r#"export function now(): number {
  return Date.now();
}
"#,
    )
}

fn noise_module(i: usize) -> String {
    let topics = [
        ("color palette session debug", "session"),
        ("string formatting trim", "format"),
        ("date locale user clock", "user"),
        ("array sort comparator stable", "sort"),
        ("number parse decimal precision", "parse"),
        ("json schema validation user", "user"),
        ("url encode decode session id", "session"),
        ("buffer slice copy auth header", "auth"),
        ("stream pipe consume token masking", "token"),
        ("regex pattern capture refund", "refund"),
        ("logger transports payment audit", "payment"),
        ("middleware request id session marker", "session"),
        ("subscription event emitter", "subscription"),
        ("cache eviction user profile entry", "user"),
        ("retry backoff auth retry policy", "auth"),
        ("file upload token placeholder", "token"),
        ("crypto random sample entropy", "crypto"),
        ("queue worker poll concurrency", "queue"),
    ];
    let (topic, keyword) = topics[i % topics.len()];
    let mut s = String::new();
    s.push_str("import { Logger } from \"../utils/logger\";\n\n");
    s.push_str(&format!(
        "// utility — {topic}\nexport interface NoiseCfg_{i} {{\n  name: string;\n  {keyword}Marker: string;\n}}\n\n"
    ));
    for j in 0..3 {
        s.push_str(&format!(
            "export function noiseFn_{i}_{j}(input: string): string {{\n  const logger = new Logger(\"noise_{i}_{j}\");\n  logger.debug(`{topic} ${{input.length}}`);\n  return input.split(\"\").reverse().join(\"\");\n}}\n\n"
        ));
    }
    s
}
