# Phone Capture for the Workshop Inventory: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A phone on the home Wi-Fi photographs a label, gets one draft per item from the Mac app's label reader, reviews the drafts, and saves them to the workshop inventory.

**Architecture:** A second HTTP listener inside the Tauri app (`src-tauri/src/phone/`), separate from the extension bridge, serves a small React page (a second Vite entry) and three token-guarded JSON routes. The routes call the same inventory functions as the Tauri commands, behind the same guards. A read cache keyed by the photo's hash prevents a second paid read when the phone reloads.

**Tech Stack:** Rust (Tauri 2, tokio), React 19 + Vite 7, `qrcode` crate 0.15 (SVG only), Node's test runner for TypeScript helpers.

**Spec:** `docs/superpowers/specs/2026-11-19-phone-inventory-capture-design.md`

## Global Constraints

- Phone port default **47822**. The bridge (port 47821, its token, its 127.0.0.1 listener, its routes) keeps its behaviour.
- Listener on `0.0.0.0` (IPv4). Off by default; on/off, port and token remembered in `AppConfig.phone`.
- Token: made like the bridge's `new_token()` (64 hex characters). It travels as `Authorization: Bearer <token>`; the page gets it from the URL fragment `#t=<token>`.
- The token is checked from the headers **before any body is read**. Limits: `/api/read` body ≤ 12 MiB (12 × 1024 × 1024); `/api/save` body ≤ 12 MiB + 512 KiB. POST without `Content-Length` → 411.
- Every answer: `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`. No `Access-Control-*` header on any answer.
- Read cache: 10 minutes, at most 4 entries. One label read at a time across the Mac and the phone.
- Phone photos: long side ≤ 3072 px, re-encoded JPEG at quality 0.9.
- Copy: use the sentences the spec quotes, verbatim.
- Phone inputs: font size ≥ 16 px; `autoCorrect="off" autoCapitalize="off" spellCheck={false}` on part number, brand and colour; `inputMode="numeric"` on counts.
- Never open `~/Corydoras` (the real library) or the real config folder from tests or builds. Rust tests that touch `AppConfig` run inside `commands_take_what_the_frontend_sends` (it sets `CORYDORAS_CONFIG_DIR`); other tests do not touch `AppConfig`.
- Start every shell command with `cd ~/Projects/corydoras` (the working directory can reset to the library).
- **Do not commit.** The designer decides when to commit. The global pre-commit hook rejects `token`/`secret`/`password` followed by `:` or `=` and a quoted string of 12+ characters, so test fixtures use short tokens (`"tok"`) or other names.

## Review Focus

1. **A phone photo in HEIC, or over 12 MiB, or 48 MP.** Expected: the page prepares a JPEG ≤ 3072 px and the read works. Pinned by `fitWithin` tests (Task 6) and a large photo in the live check (Task 9).
2. **The page reloads, or the phone locks, during a read.** Expected: the review comes back and no second provider call is made. Pinned by the cache tests (Task 3) and the end-to-end double read (Task 5).
3. **The Mac and the phone read at the same moment.** Expected: the second gets "Wait for the current label to finish." Pinned in Task 2.
4. **The inventory module is turned off, or the library is moving, while the phone has cards.** Expected: save answers with the server's message and the cards stay. Pinned in Task 5 (module off) and Task 7 (cards kept on error).
5. **A path outside the phone entry is requested** (`/index.html`, `/assets/../x`, an unknown asset that Tauri's resolver would answer with `index.html`). Expected: 404, never the desktop shell. Pinned in Task 4.

---

### Task 1: Shared HTTP request reading, headers first

**Files:**
- Create: `src-tauri/src/http.rs`
- Modify: `src-tauri/src/bridge.rs` (replace `Request`, `read_request`, `same`; keep `MAX_BODY`, `response`, routes)
- Modify: `src-tauri/src/main.rs` (`mod http;`)

**Interfaces:**
- Produces:
  - `pub(crate) struct Head { pub method: String, pub path: String, pub bearer: String, pub origin: String, pub content_length: Option<usize>, pub chunked: bool }`
  - `pub(crate) async fn read_head(stream: &mut tokio::net::TcpStream) -> Option<(Head, Vec<u8>)>`: reads up to the blank line (≤ 32 KiB headers, 10 s per read); returns the bytes already read past the head.
  - `pub(crate) async fn read_body(stream: &mut tokio::net::TcpStream, already: Vec<u8>, length: usize) -> Option<Vec<u8>>`: 30 s per read; `None` when the peer closes early.
  - `pub(crate) fn same_token(a: &str, b: &str) -> bool`: the bridge's constant-time compare, moved.

- [ ] **Step 1: Write the failing tests** in `http.rs` (`#[cfg(test)] mod tests`, run futures with `tauri::async_runtime::block_on`, connect a client to a `tokio::net::TcpListener` on `127.0.0.1:0`):
  - `head_comes_back_without_the_body`: client writes `POST /api/read HTTP/1.1\r\nAuthorization: Bearer tok\r\nContent-Length: 10485760\r\n\r\n` and nothing else, keeps the socket open; `read_head` returns within 2 s with `method == "POST"`, `path == "/api/read"`, `bearer == "tok"`, `content_length == Some(10485760)`, `already.is_empty()`.
  - `bytes_after_the_head_are_kept`: head plus `abc` in one write; `already == b"abc"`; `read_body(.., already, 5)` after the client writes `de` returns `b"abcde"`.
  - `chunked_is_flagged`: `Transfer-Encoding: chunked` → `chunked == true`, `content_length == None`.
  - `oversized_headers_are_refused`: 40 KiB of header lines → `None`.
  - `same_token_compares_whole_strings`: `("abc","abc")` true; `("abc","abd")`, `("abc","ab")`, `("","x")` false.
- [ ] **Step 2: Run them to see them fail**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras http::`
  Expected: compile error, `read_head` not found.
- [ ] **Step 3: Implement `http.rs`** with the signatures above. Header names are case-insensitive; `bearer` is the value after `Bearer ` or empty.
- [ ] **Step 4: Move the bridge onto it.** `serve` in `bridge.rs`: `read_head` → `OPTIONS` answers 204 as before → wrong token 401 as before → `content_length` over `MAX_BODY` or missing on POST → 400 "The request is not readable or too large." → `read_body` → `route`. `route` takes `(method, path, body)` instead of `&Request`.
- [ ] **Step 5: Run all Rust tests**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras`
  Expected: all pass, including the new `http::tests`.

---

### Task 2: One set of inventory guards for the Mac and the phone

**Files:**
- Modify: `src-tauri/src/inventory.rs`
- Modify: `src-tauri/src/main.rs:67` (`lib` takes `&AppState`; callers that pass `&State<AppState>` still compile through deref)
- Test: `src-tauri/src/ipc_tests.rs` (inside `commands_take_what_the_frontend_sends`)

**Interfaces:**
- Produces:
  - `pub(crate) fn enabled_library(state: &AppState) -> CmdResult<Arc<Library>>`: the move check and the module-on check, unchanged messages.
  - `pub(crate) static READER: tokio::sync::Semaphore` (1 permit), moved out of `read_inventory_label`.
  - `pub(crate) async fn read_label(library: Arc<Library>, photo: Vec<u8>) -> CmdResult<Vec<InventoryDraft>>`: size check (nonempty, ≤ `MAX_LABEL_BYTES`, message "Select a nonempty photo of 12 MiB or less."), `READER.try_acquire()` ("Wait for the current label to finish."), effective `AiConfig`, `inventory_label::read_labels`.
  - `pub(crate) async fn save_batch_frame(library: Arc<Library>, frame: Vec<u8>) -> CmdResult<Vec<InventoryItem>>`: `unpack_frame::<BatchForm>`, `DECODER`, `save_inventory_batch` on a blocking thread.
  - `read_inventory_label` and `save_inventory_batch` (the commands) become thin wrappers over these two.

- [ ] **Step 1: Write the failing test.** In `commands_take_what_the_frontend_sends`, before `inventory_photo::reviewed_record`: hold `let held = inventory::READER.try_acquire().unwrap();`, send `read_inventory_label` with the `PNG` bytes, assert the error is exactly `"Wait for the current label to finish."`, then `drop(held)`.
- [ ] **Step 2: Run it to see it fail**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras commands_take_what_the_frontend_sends`
  Expected: compile error, `READER` is not a member of `inventory`.
- [ ] **Step 3: Implement the five items above.** No message changes.
- [ ] **Step 4: Run all Rust tests**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras`
  Expected: all pass.

---

### Task 3: The read cache

**Files:**
- Create: `src-tauri/src/phone/mod.rs` (for now only `mod reads;`), `src-tauri/src/phone/reads.rs`
- Modify: `src-tauri/src/main.rs` (`mod phone;`)

**Interfaces:**
- Produces:
  - `pub(crate) struct ReadCache` with `pub(crate) fn new(ttl: Duration, capacity: usize) -> Arc<Self>`.
  - `pub(crate) async fn get_or_read<F, Fut>(self: &Arc<Self>, photo: &[u8], read: F) -> Result<Vec<InventoryDraft>, String> where F: FnOnce() -> Fut, Fut: Future<Output = Result<Vec<InventoryDraft>, String>> + Send + 'static`
  - Key: `corydoras_core::util::fnv1a64(photo)`. A running read is a spawned task (`tauri::async_runtime::spawn`) that stores its result itself, so a caller that goes away does not cancel it; later callers wait on a `tokio::sync::watch` for that key. Errors are handed to the waiters but not kept. When full, the oldest finished entry goes first.
  - `pub(crate) const READ_TTL: Duration = Duration::from_secs(600); pub(crate) const READ_CAPACITY: usize = 4;`

- [ ] **Step 1: Write the failing tests** in `reads.rs` (`block_on`; a counting `read` closure that sleeps 100 ms and returns one draft named after the call number):
  - `same_photo_while_running_reads_once`: two `get_or_read` for the same bytes joined together → both get the draft named `1`; counter is 1.
  - `same_photo_after_it_finished_reads_once`: sequential calls → counter 1, same draft.
  - `different_photos_read_separately`: counter 2.
  - `an_expired_result_is_read_again`: ttl 50 ms, sleep 80 ms between calls → counter 2.
  - `a_caller_that_leaves_does_not_cancel_the_read`: start a call, drop its future after 10 ms, wait 200 ms, call again → counter 1, the draft comes back at once (< 50 ms).
  - `a_failed_read_is_not_kept`: first read returns `Err("provider down")` → caller gets that error; next call reads again → counter 2.
  - `the_oldest_finished_entry_goes_first`: capacity 2; photos A, B, C; then A again → counter 4; then C again → counter stays 4.
- [ ] **Step 2: Run them to see them fail**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras phone::reads`
  Expected: compile error, `ReadCache` not found.
- [ ] **Step 3: Implement `ReadCache`** as described.
- [ ] **Step 4: Run them to see them pass**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras phone::reads`
  Expected: 7 passed.

---

### Task 4: The phone HTTP server

**Files:**
- Create: `src-tauri/src/phone/server.rs`
- Modify: `src-tauri/src/phone/mod.rs` (`mod server;`)

**Interfaces:**
- Consumes: `http::{read_head, read_body, same_token, Head}` (Task 1).
- Produces:
  - `pub(crate) type Reply = Result<serde_json::Value, (u16, String)>;`
  - `pub(crate) type BoxFut<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;`
  - ```rust
    pub(crate) trait Backend: Send + Sync + 'static {
        fn status(&self) -> Reply;
        fn read(&self, photo: Vec<u8>) -> BoxFut<Reply>;
        fn save(&self, frame: Vec<u8>) -> BoxFut<Reply>;
        /// Bytes and MIME type of a built file, by its path in `dist/` ("phone.html", "assets/x.js").
        fn asset(&self, path: &str) -> Option<(Vec<u8>, String)>;
    }
    ```
  - `pub(crate) async fn serve<B: Backend>(stream: tokio::net::TcpStream, backend: Arc<B>, token: Arc<String>)`
  - `pub(crate) const MAX_READ_BODY: usize = 12 * 1024 * 1024; pub(crate) const MAX_SAVE_BODY: usize = MAX_READ_BODY + 512 * 1024;`

Routes, checked in this order:

| Request | Answer |
|---|---|
| `GET /` | `asset("phone.html")`, `text/html; charset=utf-8` |
| `GET /assets/<name>`, `<name>` matches `^[A-Za-z0-9._-]+\.(js|css)$` and contains no `..` | `asset("assets/<name>")` with its MIME type; 404 if missing **or if the MIME type starts with `text/html`** (Tauri falls back to `index.html`) |
| any other `GET` that is not `/api/status` | 404 |
| no token or wrong token (any `/api/*`) | 401 `{"error":"This phone is no longer connected. Open Corydoras on your Mac, go to Settings › Phone, and scan the code."}`, body not read |
| `GET /api/status` | `backend.status()` |
| `POST /api/read`, `POST /api/save` without `Content-Length` (or chunked) | 411 `{"error":"Send the photo with a Content-Length."}` |
| … with a length over `MAX_READ_BODY` / `MAX_SAVE_BODY` | 413 `{"error":"The photo is larger than 12 MiB."}`, body not read |
| `POST /api/read` | `backend.read(body)` |
| `POST /api/save` | `backend.save(body)` |
| anything else (including `OPTIONS`) | 405 `{"error":"Not allowed."}` |

`Err((status, text))` from the backend becomes that status with `{"error": text}`. Reason phrases for 200, 204, 400, 401, 404, 405, 411, 413, 500.

- [ ] **Step 1: Write the failing tests** in `server.rs` with a `FakeBackend` (records the bytes it got; `asset` returns `("<html>phone</html>", "text/html")` for `phone.html`, `("x", "text/javascript")` for `assets/app.js`, and `("<html>desktop</html>", "text/html")` for every other path, imitating Tauri's fallback). Each test runs `serve` for one connection on `127.0.0.1:0` with token `"tok"` and reads the whole answer as text:
  - `no_token_is_refused_before_the_body`: head with `Content-Length: 10485760` and no body, socket kept open → answer starts `HTTP/1.1 401` within 2 s; `FakeBackend` got nothing.
  - `wrong_token_is_refused`: `Bearer nope` on `GET /api/status` → 401.
  - `an_oversize_read_is_413_before_the_body`: token ok, `Content-Length: 12582913`, no body → 413 within 2 s.
  - `a_post_without_length_is_411`.
  - `read_and_save_pass_the_body`: `POST /api/read` body `abc` → `FakeBackend` got `abc` for read; same for save.
  - `the_shell_is_public_and_has_the_safety_headers`: `GET /` without token → 200, body `<html>phone</html>`, headers contain `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, and no `Access-Control-`.
  - `only_phone_files_are_served`: `/assets/app.js` → 200; `/assets/missing.js` → 404 (fallback HTML refused); `/assets/../index.html`, `/assets/a.html`, `/index.html` → 404.
  - `options_gets_no_cors`: `OPTIONS /api/read` with `Origin: http://evil.example` → 405, no `Access-Control-`.
  - `backend_errors_keep_their_status`: `status()` returns `Err((400, "Workshop inventory is off for this library. Open Settings to turn it on."))` → 400 with that text in `error`.
- [ ] **Step 2: Run them to see them fail**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras phone::server`
  Expected: compile error, `serve` not found.
- [ ] **Step 3: Implement `server.rs`** to the table.
- [ ] **Step 4: Run them to see them pass**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras phone::server`
  Expected: 9 passed.

---

### Task 5: The phone link in the app

**Files:**
- Modify: `crates/corydoras-core/src/config.rs` (add `PhoneConfig`, `DEFAULT_PHONE_PORT`, `AppConfig.phone`)
- Modify: `src-tauri/src/phone/mod.rs` (state, listener, `AppBackend`, commands, address, QR)
- Modify: `src-tauri/src/main.rs` (`AppState.phone: phone::PhoneState`; `phone::start(app.handle())` after `bridge::start`; register `phone::get_phone`, `phone::set_phone`)
- Modify: `src-tauri/Cargo.toml` (`qrcode = { version = "0.15", default-features = false, features = ["svg"] }`; confirm the version resolves offline-free with `cargo tree -p qrcode`)
- Modify: `src/api.ts` (add `PhoneView`, `getPhone`, `setPhone`)
- Create: `src-tauri/src/ipc_tests/phone.rs`; Modify: `src-tauri/src/ipc_tests.rs` (call it from `commands_take_what_the_frontend_sends`; add the two commands to `app_with`)

**Interfaces:**
- Consumes: `inventory::{enabled_library, read_label, save_batch_frame}` (Task 2), `ReadCache` (Task 3), `server::{serve, Backend, Reply, BoxFut}` (Task 4), `bridge::new_token` (make it `pub(crate)`).
- Produces:
  - Core: `pub const DEFAULT_PHONE_PORT: u16 = 47822;` `#[derive(Serialize, Deserialize, Clone, Debug)] pub struct PhoneConfig { #[serde(default)] pub enabled: bool, #[serde(default = "default_phone_port")] pub port: u16, #[serde(default)] pub token: String }` with `Default` (off, 47822, empty); `#[serde(default)] pub phone: PhoneConfig` on `AppConfig`.
  - `#[derive(Serialize, Clone)] pub struct PhoneView { pub enabled: bool, pub port: u16, pub running: bool, pub error: Option<String>, pub url: Option<String>, pub local_url: Option<String>, pub qr_svg: Option<String> }` (`url`, `local_url`, `qr_svg` only while running).
  - `#[tauri::command] pub fn get_phone(app: AppHandle) -> PhoneView`
  - `#[tauri::command] pub fn set_phone(app: AppHandle, enabled: bool, port: u16, new_token_now: bool) -> CmdResult<PhoneView>`; makes a token when empty or asked; restarts the listener.
  - `pub fn start(app: &AppHandle)`: like `bridge::start`, binding `("0.0.0.0", port)`; error text `"Port {port} is not free ({e}). Choose another port."`; each connection runs `serve` inside a 330 s timeout.
  - `fn check_port(port: u16, bridge_port: u16) -> Result<(), String>`: below 1024 → `"Choose a port from 1024 to 65535."`; equal to the bridge's → `"The browser extension uses port {bridge_port}. Choose another port."`
  - `fn phone_url(ip: Ipv4Addr, port: u16, token: &str) -> String` → `http://{ip}:{port}/#t={token}`; `fn local_url(host: &str, port: u16) -> String` → `http://{host}.local:{port}`
  - `fn lan_ipv4() -> Option<Ipv4Addr>`: a `std::net::UdpSocket` bound to `0.0.0.0:0`, `connect("1.1.1.1:9")` (UDP connect sends no packet), `local_addr()`; `None` for loopback or unspecified.
  - `fn local_host_name() -> Option<String>`: `scutil --get LocalHostName`, trimmed, non-empty.
  - `fn qr_svg(text: &str) -> Option<String>`: `qrcode::QrCode::new`, rendered with `qrcode::render::svg::Color`, min dimension 240, quiet zone on.
  - `pub(crate) struct AppBackend { app: AppHandle, reads: Arc<ReadCache> }` with `pub(crate) fn new(app: AppHandle) -> Arc<Self>` and `impl Backend`:
    - `status` → `{"app":"Corydoras","version":…,"library":<library folder name>,"inventory_enabled":bool,"provider_ready":bool,"categories":[sorted, unique]}`. `provider_ready` is `AiConfig::effective(..).for_role(Role::Analysis).is_some()`. No library → `Err((400, "Open a library in Corydoras first."))`. `categories` is `[]` when the module is off.
    - `read` → `enabled_library` (Err → 400), then `reads.get_or_read(&photo, move || read_label(lib, photo))` → `{"drafts": […]}`; Err → 400.
    - `save` → `enabled_library` (Err → 400), `save_batch_frame` → emit `library-changed` with `""` and `phone-added` with `{"count": n, "library": <folder name>}` → `{"items": […]}`; Err → 400.
    - `asset` → `app.asset_resolver().get(path.to_string()).map(|a| (a.bytes, a.mime_type))`.
  - `src/api.ts`: `export interface PhoneView { enabled: boolean; port: number; running: boolean; error: string | null; url: string | null; local_url: string | null; qr_svg: string | null }`; `getPhone: () => invoke<PhoneView>('get_phone')`, `setPhone: (enabled: boolean, port: number, newTokenNow: boolean) => invoke<PhoneView>('set_phone', { enabled, port, newTokenNow })`.

- [ ] **Step 1: Write the failing unit tests** in `phone/mod.rs`:
  - `phone_url_keeps_the_token_in_the_fragment`: `phone_url(192.168.1.20, 47822, "tok") == "http://192.168.1.20:47822/#t=tok"`.
  - `local_url_adds_dot_local`: `local_url("Studio-Mac", 47822) == "http://Studio-Mac.local:47822"`.
  - `ports_below_1024_and_the_bridge_port_are_refused`: `check_port(80, 47821)` and `check_port(47821, 47821)` are `Err` with the texts above; `check_port(47822, 47821)` is `Ok`.
  - `the_qr_code_is_an_svg`: `qr_svg("http://192.168.1.20:47822/#t=tok")` starts with `<?xml` or `<svg` and contains `</svg>`.
  - Core, in `config.rs` tests: `an_old_config_gets_the_phone_defaults`: `serde_json::from_str::<AppConfig>("{}")` → `phone.enabled == false`, `phone.port == 47822`.
- [ ] **Step 2: Write the failing end-to-end test** `ipc_tests/phone.rs`, `pub(super) fn phone_link(app: &tauri::App<MockRuntime>, w: &WebviewWindow<MockRuntime>, library: &Library)`, called after `reviewed_record` (the config dir is already the test's own):
  - `call(w, "get_phone", json!({}))` → `enabled == false`, `running == false`.
  - `call(w, "set_phone", json!({"enabled": false, "port": 80, "newTokenNow": false}))` → error `"Choose a port from 1024 to 65535."`.
  - Set up the stand-in provider exactly as `reviewed_record` does (reuse `serve_label`, make it `pub(super)`). Build `let backend = phone::AppBackend::new(app.handle().clone());`. `block_on(backend.read(png.clone()))` twice → both `Ok` with one draft named `"Hex key"`; the stand-in served one connection only (join its thread after the first read; the second read must not connect, so bind the listener for one `accept` and assert `library.usage().requests` rose by exactly 1).
  - Save: frame `{"drafts":[draft with "quantity":3]}` + `png` → `Ok` with one item; `list_inventory` now contains an item named `"Hex key"` with `quantity == 3`.
  - Module off: `set_inventory_enabled(false)`; `backend.save(frame)` → `Err((400, "Workshop inventory is off for this library. Open Settings to turn it on."))`; `backend.status()` → `inventory_enabled == false`, `categories == []`. Turn it back on.
- [ ] **Step 3: Run them to see them fail**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras-core config && cargo test -p corydoras`
  Expected: compile errors for the missing items.
- [ ] **Step 4: Implement** everything in the Interfaces block.
- [ ] **Step 5: Run all Rust tests**
  Run: `cd ~/Projects/corydoras && cargo test -p corydoras-core && cargo test -p corydoras`
  Expected: all pass.

---

### Task 6: Phone page foundation (build entry, client, photo preparation, saved review)

**Files:**
- Create: `phone.html` (repo root, beside `index.html`; viewport `width=device-width, initial-scale=1, viewport-fit=cover`; `<meta name="referrer" content="no-referrer">`; loads `/src/phone/main.tsx`)
- Modify: `vite.config.ts` (`build.rollupOptions.input: { main: 'index.html', phone: 'phone.html' }`)
- Create: `src/inventoryFrame.ts`; Modify: `src/inventoryApi.ts` (`saveBatch` uses it)
- Create: `src/phone/phoneApi.ts`, `src/phone/photo.ts`, `src/phone/session.ts`
- Test: `tests/workshop-phone.test.mjs` (picked up by `npm run test:workshop`)

**Interfaces:**
- Produces:
  - `src/inventoryFrame.ts`: `export async function frameDrafts(drafts: InventoryDraft[], photo: Blob): Promise<Uint8Array>` (4-byte big-endian JSON length, JSON `{ drafts }`, photo bytes). No Tauri import.
  - `src/phone/phoneApi.ts` (no Tauri import):
    - `export const TOKEN_KEY = 'corydoras.phone.token'`
    - `export function tokenFromHash(hash: string): string | null` (`#t=<token>` only)
    - `export function connect(): string | null`: a token in `location.hash` is stored under `TOKEN_KEY` and the hash removed with `history.replaceState`; otherwise the stored token.
    - `export interface PhoneStatus { app: string; version: string; library: string; inventory_enabled: boolean; provider_ready: boolean; categories: string[] }`
    - `export class PhoneError extends Error { constructor(message: string, readonly status: number | 'offline') }`
    - `export const OFFLINE = 'Cannot reach Corydoras. Check that the Mac is awake, on the same Wi-Fi, and that Phone is on in Settings.'`
    - `export const DISCONNECTED = 'This phone is no longer connected. Open Corydoras on your Mac, go to Settings › Phone, and scan the code.'`
    - `export function messageFor(status: number | 'offline', serverText?: string): string`: `'offline'` → `OFFLINE`; 401 → `DISCONNECTED` (and the caller clears the token); otherwise `serverText` or `Corydoras answered with an error (${status}).`
    - `export const phoneApi = { status(): Promise<PhoneStatus>; read(photo: Blob): Promise<InventoryDraft[]>; save(drafts: InventoryDraft[], photo: Blob): Promise<InventoryItem[]> }`: `fetch` with `Authorization: Bearer <token>`; a network failure throws `PhoneError(OFFLINE, 'offline')`; a 401 removes `TOKEN_KEY`.
  - `src/phone/photo.ts`: `export function fitWithin(width: number, height: number, max: number): { width: number; height: number }` (rounded, never upscales); `export async function preparePhoto(file: Blob): Promise<Blob>`: `createImageBitmap(file, { imageOrientation: 'from-image' })`, canvas at `fitWithin(.., 3072)`, `toBlob('image/jpeg', 0.9)`; a decode failure throws `Error('This photo cannot be read. Take it again or choose a JPEG.')`.
  - `src/phone/session.ts`: `export interface PhoneSession { photo: Blob; cards: WorkshopForm[] | null }`; `loadSession(): Promise<PhoneSession | null>`, `saveSession(s: PhoneSession): Promise<void>`, `clearSession(): Promise<void>` (IndexedDB database `corydoras-phone`, store `session`, key `current`). `cards === null` means a read was running.

- [ ] **Step 1: Write the failing tests** in `tests/workshop-phone.test.mjs`:
  - `tokenFromHash`: `'#t=abc'` → `'abc'`; `'#t='`, `'#x=1'`, `''` → `null`.
  - `fitWithin`: `(4032, 3024, 3072)` → `{ width: 3072, height: 2304 }`; `(3024, 4032, 3072)` → `{ width: 2304, height: 3072 }`; `(1000, 800, 3072)` → `{ width: 1000, height: 800 }`.
  - `frameDrafts`: for one draft and a 3-byte `Blob`, the first 4 bytes read big-endian equal the JSON length; the JSON parses to `{ drafts: [draft] }`; the last 3 bytes equal the photo.
  - `messageFor`: `'offline'` → `OFFLINE`; `401` → `DISCONNECTED`; `(400, 'Select an image analysis provider in Settings.')` → that text; `500` with no text → `'Corydoras answered with an error (500).'`.
- [ ] **Step 2: Run them to see them fail**
  Run: `cd ~/Projects/corydoras && npm run test:workshop`
  Expected: fails, module `src/phone/phoneApi.ts` not found.
- [ ] **Step 3: Implement** the files above; `phone.html` mounts an empty root for now (`main.tsx` renders `null` until Task 7).
- [ ] **Step 4: Run the tests and the type check**
  Run: `cd ~/Projects/corydoras && npm run test:workshop && npx tsc -p tsconfig.json --noEmit`
  Expected: all tests pass; no type errors.
- [ ] **Step 5: Check the build has both entries**
  Run: `cd ~/Projects/corydoras && npx vite build --outDir "$TMPDIR/cory-phone-dist" --emptyOutDir && ls "$TMPDIR/cory-phone-dist"`
  Expected: `index.html`, `phone.html`, `assets/`.

---

### Task 7: The phone screens

**Files:**
- Create: `src/phone/main.tsx`, `src/phone/PhoneApp.tsx`, `src/phone/PhoneReview.tsx`, `src/phone/phone.css`

**Interfaces:**
- Consumes: `phoneApi`, `connect`, `messageFor`, `PhoneError`, `TOKEN_KEY` (Task 6); `preparePhoto`; `loadSession`/`saveSession`/`clearSession`; from `src/components/workshopForm.ts`: `editForm`, `blankForm`, `addPack`, `batchDrafts`, `specSummary`, `withSpecs`, `removeSpec`, `LABEL_CATEGORIES`; `sourceLabel` from `src/components/WorkshopSpecsTable.tsx` (move it to `workshopForm.ts` if importing the component file would pull in desktop code).
- Produces: `PhoneApp` with the states `connect | ready | reading | review | saved`.

Copy (verbatim from the spec):
- Connect: "Open Corydoras on your Mac, go to Settings › Phone, and scan the code."
- Reading: "Reading the label… this can take up to a minute."
- Saved: "{n} items saved to {library}." (`1 item saved to {library}.` for one) and the button **Take another photo**.
- Buttons: **Take photo** (`<input type="file" accept="image/*" capture="environment">`), **Choose photo** (same without `capture`), **I bought one pack**, **Discard**, **Add an item by hand**, **Save N items** (`Save 1 item`), **Cancel**.

Behaviour:
- [ ] **Step 1: Ready and connect.** On load, `connect()`; no token → connect screen. Else `phoneApi.status()`; show the library name; if `!inventory_enabled` show the server's module-off text and disable the photo buttons; if `!provider_ready` show "No analysis provider is set on the Mac. You can still add items by hand." and keep the buttons.
- [ ] **Step 2: Photo and read.** On a file: `preparePhoto` → `saveSession({ photo, cards: null })` → reading screen → `phoneApi.read` → `saveSession({ photo, cards })` → review. A read error shows its message on the review screen with zero cards, so **Add an item by hand** is the way forward.
- [ ] **Step 3: Review cards** (`PhoneReview`): one card per form with the fields in the spec order; category with a `<datalist>` of `LABEL_CATEGORIES` plus `status.categories`; the Dimensions line read-only when specs exist (`specSummary`); each spec row shows `specSummary([spec])` and `sourceLabel[spec.source]` with a **Remove** button (`withSpecs(form, removeSpec(form.specs, i))`); **I bought one pack** uses `addPack` and shows its error; **Discard** removes the card. Every change calls `saveSession`. The save bar is fixed at the bottom; **Cancel** with cards asks inline ("Discard the N unsaved items from this photo?" / **Keep reviewing** / **Discard items**).
- [ ] **Step 4: Save.** `batchDrafts(forms)` (its error shows above the save bar) → `phoneApi.save(drafts, photo)` → `clearSession()` → saved screen. On error the cards stay and the message shows; 401 goes to the connect screen with `DISCONNECTED`.
- [ ] **Step 5: Reload.** On load with a session: `cards !== null` → review; `cards === null` → reading screen and `phoneApi.read(photo)` again (the cache answers).
- [ ] **Step 6: Phone CSS.** `min-height: 100dvh`; `padding: env(safe-area-inset-*)` on the page and the save bar; inputs `font-size: 16px`; `-webkit-tap-highlight-color: transparent`; buttons `user-select: none`; light and dark colours from `prefers-color-scheme`, matching the desktop palette tokens in `src/styles.css` where they exist.
- [ ] **Step 7: Type check and build**
  Run: `cd ~/Projects/corydoras && npx tsc -p tsconfig.json --noEmit && npm run test:workshop && npx vite build --outDir "$TMPDIR/cory-phone-dist" --emptyOutDir`
  Expected: no errors; build succeeds.

---

### Task 8: The Mac side (Settings › Phone, toast, list refresh, README)

**Files:**
- Create: `src/components/PhonePanel.tsx`
- Modify: `src/components/SettingsView.tsx` (render `PhonePanel` right after the inventory switch, only when `info.inventory_enabled`)
- Modify: `src/App.tsx` (listen to `phone-added`; pass `tick` to `WorkshopInventory`)
- Modify: `src/components/WorkshopInventory.tsx` (prop `tick: number`; reload the list when it changes)
- Modify: `README.md` (a "Phone" paragraph in the workshop inventory section)

**Interfaces:**
- Consumes: `api.getPhone`, `api.setPhone`, `PhoneView` (Task 5); `useConfirmRow`; `useToast`.

- [ ] **Step 1: `PhonePanel`.** Modelled on `BridgePanel`. Switch label "Let my phone add inventory items over Wi-Fi"; port field; when running: the QR code (`qr_svg` inserted as markup: it is generated by the app, not user text), the link with **Copy**, `local_url` as text with the note "Some phones cannot open .local addresses; use the link above if this one fails.", **Refresh** (calls `getPhone`), **New token** with the inline two-step confirm ("Phones that scanned the old code are disconnected."). Error box for `error`. When on but `url` is null: "This Mac has no Wi-Fi address right now." The explanatory text, verbatim: "While this is on, any device on the same network as this Mac can open the page, wherever the Mac is. Only a phone that scanned this code can read labels or save. The connection is not encrypted, so use it on a network you trust. Each read uses your analysis provider and counts toward the monthly budget, like a read on the Mac."
- [ ] **Step 2: Toast.** In `App.tsx`, `listen<{ count: number; library: string }>('phone-added', …)` → `toast.info(count === 1 ? '1 item added from your phone.' : \`${count} items added from your phone.\`)`.
- [ ] **Step 3: List refresh.** `WorkshopInventory` adds `tick` to the dependency list of the effect that calls `inventoryApi.list()`, and does not set `loading` on a tick reload (no flash). `editor` and `review` state are untouched by it.
- [ ] **Step 4: README.** Two to four sentences: what Settings › Phone does, that the Mac must be awake on the same Wi-Fi, and that the connection is plain HTTP for a trusted network.
- [ ] **Step 5: Type check, tests, build**
  Run: `cd ~/Projects/corydoras && npx tsc -p tsconfig.json --noEmit && npm run test:workshop && cargo test -p corydoras`
  Expected: all pass.

---

### Task 9: Live check on a library copy

Follow the `corydoras-live-check` recipe; nothing here touches `~/Corydoras`, the real config, or the installed app.

- [ ] **Step 1: Library copy and scratch config.** `sqlite3 -readonly ~/Corydoras/library.sqlite ".backup '<scratch>/lib/library.sqlite'"` and `cp -cR ~/Corydoras/languages <scratch>/lib/`. Config dir `<scratch>/config` (no provider keys, so no paid read can run).
- [ ] **Step 2: DevCheck build.** `npx tauri build --debug --bundles app --config '{"identifier":"com.travisforsyth.corydoras.devcheck","productName":"Corydoras DevCheck"}'`; run its binary with `CORYDORAS_LIBRARY=<scratch>/lib CORYDORAS_CONFIG_DIR=<scratch>/config`.
- [ ] **Step 3: Turn the phone link on** in Settings › Phone with port **47832**; check the QR code, the link and the `.local` address show.
- [ ] **Step 4: Phone check.** Open the LAN link (not `localhost`) in iOS Simulator Safari, which shares the Mac's network, at iPhone size. Check: connect screen without a token; ready screen with the token; a 4032 × 3024 photo from the simulator's library goes through preparation; the read fails with the no-provider message; **Add an item by hand** → fill a name → **Save 1 item** → saved screen; reload in the middle of a review restores it.
- [ ] **Step 5: Mac check.** The toast "1 item added from your phone." shows; the inventory list shows the item with its photo; an item editor left open on the Mac during the phone save keeps its typed text.
- [ ] **Step 6: Disconnect checks.** **New token** → the phone's next action shows `DISCONNECTED` and the connect screen. Quit the DevCheck app → the phone shows `OFFLINE` and keeps its cards.
- [ ] **Step 7: Clean up** per the recipe: kill the app by PID; delete `target/debug/bundle/macos/Corydoras DevCheck.app`, `~/Library/{WebKit,Caches}/com.travisforsyth.corydoras.devcheck`, the DevCheck preferences plist (`defaults delete` first), and the scratch library and config.
- [ ] **Step 8: Report** what only the designer can check: a real phone camera on real Wi-Fi (which file format the phone sends) and a paid read from the phone, which runs only with the designer's OK.
