//! Token comparison: `pluck.search` (BM25 top-K chunks) vs bash workflows.
//!
//! Models a synthetic multi-file repo, runs realistic agent queries through
//! each retrieval strategy, and reports cl100k_base token counts for each.
//! Mirrors the methodology used in similar tools, but with tiktoken
//! (not the 4-chars-per-token approximation) for accuracy.

use pluck_core::chunker::{chunk_source, Language};
use pluck_core::index::{PluckIndex, SearchHit};
use tiktoken_rs::cl100k_base;

// ── Synthetic repo ──────────────────────────────────────────────────────────

fn repo() -> Vec<(String, String)> {
    let mut r: Vec<(String, String)> = vec![
        ("src/auth/session.ts".into(), session_ts()),
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
    // Add noise files: unrelated utility/types/config modules that mention some
    // common English words. Real repos look like this — most files are
    // tangential to any given query. Without noise, the grep+cat baseline is
    // artificially tight because every file is relevant.
    for i in 0..80 {
        let path = format!("src/noise/mod_{i}.ts");
        r.push((path, noise_module(i)));
    }
    r
}

fn noise_module(i: usize) -> String {
    // Each noise module mentions one or two query-ish keywords incidentally —
    // a log line, a comment, a variable name. Models real codebases where
    // common nouns ("session", "user", "auth") appear in tangentially related
    // files (logging modules, generic middleware, type definitions).
    //
    // grep-by-substring pulls every one of these in; pluck.search ranks them
    // far below the actual subject-matter chunks via BM25.
    let topics = [
        ("color palette session debug", "session"),
        ("string formatting trim", "format"),
        ("date locale user clock", "user"),
        ("array sort comparator stable", "sort"),
        ("number parse decimal precision", "parse"),
        ("json schema validation user", "user"),
        ("url encode decode session id", "session"),
        ("buffer slice copy auth header", "auth"),
        ("stream pipe consume password masking", "password"),
        ("regex pattern capture refund", "refund"),
        ("logger transports payment audit", "payment"),
        ("middleware request id session marker", "session"),
        ("subscription event emitter", "subscription"),
        ("cache eviction user profile entry", "user"),
        ("retry backoff auth retry policy", "auth"),
        ("file upload password placeholder", "password"),
    ];
    let (topic, keyword) = topics[i % topics.len()];
    let mut s = String::new();
    s.push_str("import { Logger } from \"../utils/logger\";\n\n");
    s.push_str(&format!(
        "// utility module — {topic}\nexport interface NoiseConfig_{i} {{\n  name: string;\n  {keyword}Marker: string;\n  flags: string[];\n}}\n\n"
    ));
    for j in 0..4 {
        s.push_str(&format!(
            "export function noiseFn_{i}_{j}(input: string, opts?: NoiseConfig_{i}): string {{\n  const logger = new Logger(\"noise_{i}_{j}\");\n  if (!input) return \"\";\n  const out = input.split(\"\").reverse().join(\"\");\n  logger.debug(`{topic} via {keyword}-marker produced ${{out.length}} bytes for ${{opts?.{keyword}Marker}}`);\n  return out;\n}}\n\n"
        ));
    }
    s
}

fn session_ts() -> String {
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

export async function isSessionExpired(s: Session): Promise<boolean> {
  if (!s) return true;
  return s.expiresAt < now();
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
export type LoginResult =
  | { ok: true; session: Session }
  | { ok: false; error: "BAD_CREDS" | "USER_DISABLED" | "RATE_LIMITED" };

const FAILED_ATTEMPTS_THRESHOLD = 5;

export async function handleLogin(db: Database, req: LoginRequest): Promise<LoginResult> {
  const user = await db.users.findByEmail(req.email);
  if (!user) return { ok: false, error: "BAD_CREDS" };
  if (user.disabled) return { ok: false, error: "USER_DISABLED" };
  if (user.failedAttempts >= FAILED_ATTEMPTS_THRESHOLD) {
    return { ok: false, error: "RATE_LIMITED" };
  }
  const ok = await verifyPassword(req.password, user.passwordHash);
  if (!ok) {
    await db.users.bumpFailedAttempts(user.id);
    return { ok: false, error: "BAD_CREDS" };
  }
  const session = await createSession(db, user.id);
  return { ok: true, session };
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

const SCRYPT_KEY_LEN = 64;

export async function hashPassword(password: string, salt: Buffer): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    scrypt(password, salt, SCRYPT_KEY_LEN, (err, derived) => {
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
  if (actual.length !== expected.length) return false;
  return timingSafeEqual(actual, expected);
}

export function validatePasswordStrength(password: string): { ok: boolean; reason?: string } {
  if (password.length < 12) return { ok: false, reason: "TOO_SHORT" };
  if (!/[A-Z]/.test(password)) return { ok: false, reason: "NO_UPPERCASE" };
  if (!/[0-9]/.test(password)) return { ok: false, reason: "NO_DIGIT" };
  if (!/[^a-zA-Z0-9]/.test(password)) return { ok: false, reason: "NO_SYMBOL" };
  return { ok: true };
}
"#,
    )
}

fn charge_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

export type ChargeRequest = { userId: string; amountCents: number; currency: "USD" | "KRW" };
export type ChargeResult = { ok: boolean; chargeId?: string; error?: string };

export async function chargeCustomer(db: Database, req: ChargeRequest): Promise<ChargeResult> {
  const card = await db.cards.primaryFor(req.userId);
  if (!card) return { ok: false, error: "NO_PAYMENT_METHOD" };
  if (req.amountCents <= 0) return { ok: false, error: "INVALID_AMOUNT" };
  const chargeId = "ch_" + crypto.randomUUID();
  await db.charges.insert({
    id: chargeId,
    userId: req.userId,
    amountCents: req.amountCents,
    currency: req.currency,
    status: "succeeded",
    createdAt: Date.now(),
  });
  return { ok: true, chargeId };
}

export async function listCharges(db: Database, userId: string): Promise<unknown[]> {
  return db.charges.findByUser(userId);
}
"#,
    )
}

fn refund_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

const REFUND_WINDOW_MS = 1000 * 60 * 60 * 24 * 30;

export async function processRefund(db: Database, chargeId: string, amountCents: number): Promise<{ ok: boolean; refundId?: string; error?: string }> {
  const charge = await db.charges.findById(chargeId);
  if (!charge) return { ok: false, error: "CHARGE_NOT_FOUND" };
  if (Date.now() - charge.createdAt > REFUND_WINDOW_MS) {
    return { ok: false, error: "REFUND_WINDOW_EXPIRED" };
  }
  if (amountCents > charge.amountCents) {
    return { ok: false, error: "REFUND_EXCEEDS_CHARGE" };
  }
  const refundId = "rf_" + crypto.randomUUID();
  await db.refunds.insert({ id: refundId, chargeId, amountCents, createdAt: Date.now() });
  return { ok: true, refundId };
}
"#,
    )
}

fn subscription_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";
import { chargeCustomer } from "./charge";

export type Subscription = { id: string; userId: string; plan: string; nextBillingAt: number };

export async function createSubscription(db: Database, userId: string, plan: string): Promise<Subscription> {
  const sub: Subscription = {
    id: "sub_" + crypto.randomUUID(),
    userId, plan,
    nextBillingAt: Date.now() + 1000 * 60 * 60 * 24 * 30,
  };
  await db.subscriptions.insert(sub);
  return sub;
}

export async function billPendingSubscriptions(db: Database): Promise<number> {
  const due = await db.subscriptions.findDue();
  let billed = 0;
  for (const sub of due) {
    const r = await chargeCustomer(db, { userId: sub.userId, amountCents: 1000, currency: "USD" });
    if (r.ok) {
      sub.nextBillingAt = Date.now() + 1000 * 60 * 60 * 24 * 30;
      await db.subscriptions.update(sub);
      billed++;
    }
  }
  return billed;
}
"#,
    )
}

fn profile_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

export type Profile = { userId: string; displayName: string; avatarUrl?: string; bio?: string };

export async function getProfile(db: Database, userId: string): Promise<Profile | null> {
  return db.profiles.findByUser(userId);
}

export async function updateProfile(db: Database, userId: string, patch: Partial<Profile>): Promise<Profile> {
  const existing = await db.profiles.findByUser(userId);
  const next = { ...existing, ...patch, userId };
  await db.profiles.upsert(next);
  return next;
}
"#,
    )
}

fn settings_ts() -> String {
    String::from(
        r#"import { Database } from "../db/pool";

export type Settings = { userId: string; theme: "light" | "dark"; emailNotifications: boolean };

export async function getSettings(db: Database, userId: string): Promise<Settings> {
  const s = await db.settings.findByUser(userId);
  if (s) return s;
  return { userId, theme: "light", emailNotifications: true };
}

export async function updateSettings(db: Database, userId: string, patch: Partial<Settings>): Promise<Settings> {
  const cur = await getSettings(db, userId);
  const next = { ...cur, ...patch, userId };
  await db.settings.upsert(next);
  return next;
}
"#,
    )
}

fn routes_ts() -> String {
    String::from(
        r#"import { handleLogin, handleLogout } from "../auth/login";
import { chargeCustomer } from "../payment/charge";
import { getProfile, updateProfile } from "../user/profile";
import { getSettings, updateSettings } from "../user/settings";

export async function dispatch(method: string, path: string, body: any, ctx: any) {
  if (method === "POST" && path === "/auth/login") return handleLogin(ctx.db, body);
  if (method === "POST" && path === "/auth/logout") return handleLogout(ctx.db, body.sessionId);
  if (method === "POST" && path === "/payment/charge") return chargeCustomer(ctx.db, body);
  if (method === "GET" && path.startsWith("/user/profile/")) {
    const id = path.split("/").pop()!;
    return getProfile(ctx.db, id);
  }
  if (method === "PATCH" && path === "/user/profile") return updateProfile(ctx.db, body.userId, body);
  if (method === "GET" && path === "/user/settings") return getSettings(ctx.db, ctx.userId);
  if (method === "PATCH" && path === "/user/settings") return updateSettings(ctx.db, ctx.userId, body);
  return { status: 404 };
}
"#,
    )
}

fn middleware_ts() -> String {
    String::from(
        r#"import { isSessionExpired } from "../auth/session";
import { Database } from "../db/pool";

export async function requireAuth(db: Database, sessionId: string): Promise<{ ok: boolean; userId?: string; error?: string }> {
  const session = await db.sessions.findById(sessionId);
  if (!session) return { ok: false, error: "NO_SESSION" };
  if (await isSessionExpired(session)) return { ok: false, error: "SESSION_EXPIRED" };
  return { ok: true, userId: session.userId };
}

export function corsHeaders(origin: string): Record<string, string> {
  return {
    "Access-Control-Allow-Origin": origin,
    "Access-Control-Allow-Methods": "GET, POST, PATCH, DELETE",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
  };
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
  audit: any;
}

export function connectDatabase(url: string): Database {
  // placeholder
  return {} as Database;
}
"#,
    )
}

fn utils_ts() -> String {
    String::from(
        r#"export function now(): number {
  return Date.now();
}

export function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

export function uuid(): string {
  return crypto.randomUUID();
}
"#,
    )
}

// ── Retrieval strategies ────────────────────────────────────────────────────

fn query_words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Bash baseline: agent runs `rg <word>` for each query word and unions the
/// matching lines (file:line:content prefix). What an unaided agent gets.
fn rg_matches(query: &str, repo: &[(String, String)]) -> String {
    let words = query_words(query);
    let mut out = String::new();
    for (path, src) in repo {
        for (i, line) in src.lines().enumerate() {
            let lower = line.to_lowercase();
            if words.iter().any(|w| lower.contains(w)) {
                out.push_str(&format!("{path}:{}:{}\n", i + 1, line));
            }
        }
    }
    out
}

/// Realistic agent flow: identify candidate files (any query word matches),
/// then `cat` each. This is what ends up in the agent context window.
fn cat_matched_files(query: &str, repo: &[(String, String)]) -> String {
    let words = query_words(query);
    let mut out = String::new();
    for (path, src) in repo {
        let lower = src.to_lowercase();
        if words.iter().any(|w| lower.contains(w)) {
            out.push_str(&format!("=== {path} ===\n"));
            out.push_str(src);
            out.push('\n');
        }
    }
    out
}

/// pluck.search rendered with full chunk bodies (verbose, equivalent to
/// a prior-art code search tool `--json` mode).
fn pluck_search_render_full(hits: &[SearchHit]) -> String {
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!(
            "{}:L{}-{} {} ({:?})\n{}\n\n",
            h.path, h.start_line, h.end_line, h.symbol, h.kind, h.content
        ));
    }
    out
}

/// `--compact` rendering: score, path:range, then only the lines inside the
/// chunk that contain a query keyword (line number + trimmed content). This
/// is the apples-to-apples comparison with a prior-art code search tool's headline `-93%` claim.
fn pluck_search_render_compact(hits: &[SearchHit], query: &str) -> String {
    let words = query_words(query);
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!(
            "{:.4}\t{}:{}-{}\n",
            h.score, h.path, h.start_line, h.end_line
        ));
        for (i, line) in h.content.lines().enumerate() {
            let lower = line.to_lowercase();
            if words.iter().any(|w| lower.contains(w)) {
                let trimmed = line.trim();
                let ln = h.start_line as usize + i;
                out.push_str(&format!("  L{ln}: {trimmed}\n"));
            }
        }
    }
    out
}

// ── Bench main ──────────────────────────────────────────────────────────────

fn main() {
    let files = repo();

    // Build index in RAM.
    let idx = PluckIndex::in_ram().expect("build index");
    {
        let mut w = idx.writer().expect("writer");
        for (path, src) in &files {
            let chunks = chunk_source(src, Language::TypeScript)
                .unwrap_or_default();
            for c in &chunks {
                w.add_chunk(path.as_str(), c).expect("add chunk");
            }
        }
        w.commit().expect("commit");
    }

    let bpe = cl100k_base().expect("bpe");
    let count = |s: &str| bpe.encode_with_special_tokens(s).len();

    let total_repo_tokens = {
        let mut s = String::new();
        for (path, src) in &files {
            s.push_str(&format!("=== {path} ===\n"));
            s.push_str(src);
            s.push('\n');
        }
        count(&s)
    };

    let queries: &[(&str, &str)] = &[
        ("session expiry refresh", "session token expiry"),
        ("password verification", "password verify"),
        ("refund window", "refund process"),
        ("subscription billing", "billing subscription"),
        ("auth middleware", "auth middleware require"),
    ];

    println!();
    println!(
        "Repo: {} files, full-repo cat = {} tokens.",
        files.len(),
        total_repo_tokens
    );
    println!();
    println!("| Query | `rg` lines | `rg+cat files` | pluck.search `--full` | pluck.search `--compact` | save vs cat | save vs rg |");
    println!("|-------|------:|------:|------:|------:|------:|------:|");

    for (label, query) in queries {
        let rg_out = rg_matches(query, &files);
        let cat_out = cat_matched_files(query, &files);
        let hits = idx
            .search_with_cutoff(query, 10, 0.12)
            .unwrap_or_default();
        let full = pluck_search_render_full(&hits);
        let compact = pluck_search_render_compact(&hits, query);

        let rg_tok = count(&rg_out);
        let cat_tok = count(&cat_out);
        let full_tok = count(&full);
        let compact_tok = count(&compact);

        let save_cat = if cat_tok > 0 {
            (100.0 * (cat_tok as f64 - compact_tok as f64) / cat_tok as f64).round() as i64
        } else {
            0
        };
        let save_rg = if rg_tok > 0 {
            (100.0 * (rg_tok as f64 - compact_tok as f64) / rg_tok as f64).round() as i64
        } else {
            0
        };

        println!(
            "| {label} | {rg_tok} | {cat_tok} | {full_tok} | {compact_tok} | {save_cat}% | {save_rg}% |"
        );
    }
    println!();
}
