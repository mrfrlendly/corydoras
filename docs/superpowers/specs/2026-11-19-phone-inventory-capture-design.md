# Phone capture for the workshop inventory

Status: draft for approval, 2026-11-19. Nothing is built yet.

## Intent and the why

The designer adds stock to the workshop inventory by photographing labels. Today that happens in the Mac app (**Workshop inventory › Add from photo**): choose a photo file, the analysis provider reads it, review one card per item, save. The phone is the better camera and is already in hand at the workbench, so the designer wants to take the photo on the phone and finish the whole flow there: photograph, review, submit.

The designer works at the workbench at home, with the Mac on the same Wi-Fi. No capture away from home is needed.

## Goals

1. From the phone's browser: take a label photo, get one draft per item from the Mac's label reader, review and edit the drafts, and save them, with the same rules and the same all-or-nothing save as the Mac app.
2. Connecting a phone takes one QR scan in Settings, and stays connected until the designer makes a new token.
3. Items saved from the phone show up in an open inventory view on the Mac without a reload, and without disturbing an editor or review that is open there.
4. Locking the phone or reloading the page during a read never costs a second paid read of the same photo, and never loses the review in progress.

## Non-goals

- Using the phone away from the home network, an offline queue, or any cloud service.
- Browsing, searching or editing existing items on the phone. (A later change could add a stock lookup.)
- The supplier search on the phone.
- HTTPS, a live camera viewfinder in the page, a service worker or an installable app. All of these need a secure context, which plain HTTP on a LAN address is not. The phone's own camera screen does the capture, and "Add to Home Screen" still gives a home-screen shortcut.
- Managing several phones separately. Every phone that scanned the current code is connected; a new token disconnects all of them.

## Anti-goals (failures even if the feature "works")

- Anyone without the token reads the inventory, saves to it, or spends provider tokens.
- A reload or a locked phone during a read starts a second paid read of the same photo.
- Something is saved without the designer's review on the phone.
- A phone save closes or overwrites an editor or review that is open on the Mac.
- The phone link weakens the extension bridge (its token, its loopback-only listener, or its routes).

## Constraints

- Everything lives in the Mac app: the library, the provider keys, the label reader (`inventory_label::read_labels`), and the batch save (`Library::save_inventory_batch`). The phone talks to the running app; when the Mac sleeps or the app is closed, the page says it cannot reach Corydoras.
- The Mac is a MacBook. While the phone link is on, it listens on whatever network the Mac is on, not only at home. The token is what protects it there.
- Plain HTTP: traffic on the Wi-Fi is not encrypted, so someone on the same network could see the token. Acceptable for a home network; the Settings panel says so.
- The macOS application firewall is off on the designer's Mac today, so no incoming-connection prompt is expected. If it is turned on, macOS asks once to allow incoming connections for Corydoras.
- `decode_photo` accepts JPEG, PNG and WebP up to 12 MiB and 50 megapixels. Phone cameras can hand over HEIC or files over 12 MiB.

## Approaches considered

| | What it is | Trade-off |
|---|---|---|
| **1 (chosen)** | A separate phone server inside the Mac app, with its own port, token and routes. The phone does the whole flow. | New code that listens on the network, protected by a token and off until turned on. |
| 2 | Open the existing extension bridge to the Wi-Fi and add the phone routes there | Less code, one token. But the bridge downloads any URL and adds references to languages; opening it widens all of that, and the extension's token would live on the phone. |
| 3 | Phone as a camera only: photos go to an inbox, review and save on the Mac | Simplest page; full specs table on the Mac. But every item still needs a walk to the Mac, which is what the designer wants to avoid. |

Also ruled out: a hosted website (the library would have to leave the Mac) and a live viewfinder (needs HTTPS certificates on the phone).

The designer chose 1 on 2026-11-19.

## The design

### The phone server (`src-tauri/src/phone.rs`)

- Its own listener on `0.0.0.0` (IPv4), default port **47822** (the bridge uses 47821). Its own settings in `AppConfig`: `phone: { enabled, port, token }`, off by default, remembered across launches like the bridge. The token is made the same way as the bridge's (64 hex characters from two random UUIDs, about 244 random bits) the first time the phone link is turned on.
- **Auth before body.** The server reads the request headers, checks the token in `Authorization: Bearer …` (constant-time compare, as the bridge does), and answers 401 before it reads any body. Bodies larger than 12 MiB plus 512 KiB of fields get 413 before they are read. A request without `Content-Length` gets 411.
- The HTTP reading code is shared with the bridge (extracted into a small module), changed so the headers are checked before the body. The bridge keeps its behaviour, its port and its token.
- No CORS headers on any answer, so another web page cannot read anything even if it guesses the address. Every answer has `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`. The page loads nothing from other hosts.
- Connection timeout: 5 minutes for a read (a provider call can take over a minute); 30 seconds for everything else.

Routes:

| Route | Token | What it does |
|---|---|---|
| `GET /` | no | The phone page shell (HTML). It holds no data. |
| `GET /assets/*` | no | The page's built JS and CSS. No data. |
| `GET /api/status` | yes | App version, library name, whether the inventory module is on, whether an analysis provider is set, and the categories in use (for the category suggestions). |
| `POST /api/read` | yes | Body: the photo bytes. Returns `{ drafts }`, one unsaved draft per item, from `read_labels`. |
| `POST /api/save` | yes | Body: the same frame the Mac app sends (4-byte length, JSON `{ drafts }`, photo bytes). Saves through `save_inventory_batch`. Returns the saved items. |

### One set of guards for both entry points

The Tauri commands `read_inventory_label` and `save_inventory_batch` and the phone routes call the same functions in `src-tauri/src/inventory.rs`. They share:

- the library-move check and the module-on check (`enabled_library`),
- the 12 MiB limit and the frame unpacking,
- the `READER` limit (one label read at a time, across the Mac and the phone; today it is a static inside the command) and the `DECODER` limit (one photo save at a time).

So the phone and the Mac can never run two paid reads at once; the second gets "Wait for the current label to finish."

### No second paid read

The server keeps the result of each read for 10 minutes, keyed by a hash of the photo bytes (at most a few entries). If the same photo is posted again:

- while its read is still running, the request waits for that read instead of starting another;
- after it finished, the request gets the kept drafts at once.

A read whose phone went away still finishes and is kept, so the page picks it up when it comes back.

### The phone page

A second Vite entry (`phone.html`, `src/phone/`), React like the desktop app, with its own phone stylesheet. It uses the form rules in `src/components/workshopForm.ts` (`editForm`, `blankForm`, `addPack`, `batchDrafts`, `specSummary`, `LABEL_CATEGORIES`), so the phone and the Mac validate drafts the same way, and the core validates again on save. Calls go through a small `fetch` client that adds the token; nothing imports Tauri.

My call: a phone review component of its own rather than reusing `WorkshopPhotoReview`. The layout differs (camera buttons, one column, a save bar fixed at the bottom, specs as a list rather than a table), and the shared rules already live in `workshopForm.ts`.

Screens:

1. **Not connected.** No token in the address and none stored: "Open Corydoras on your Mac, go to Settings › Phone, and scan the code."
2. **Ready.** Library name. Two buttons: **Take photo** (opens the camera: `capture="environment"`) and **Choose photo** (photo library). If the inventory module is off or no provider is set, it says so here.
3. **Reading.** The photo, and "Reading the label… this can take up to a minute."
4. **Review.** One card per item: item name, category (with suggestions), brand, part number, colour, quantity on hand, quantity per pack, **I bought one pack**, the Dimensions line (generated from the specs when there are any), the specs with a **Remove** button on each row (see "Specs on the phone"), and **Discard**. Below the cards, **Add an item by hand** adds a blank card that shares the photo; this is also the way forward when the read fails or finds nothing. A bar fixed at the bottom holds **Save N items**. **Cancel** asks before discarding unsaved cards.
5. **Saved.** "3 items saved to <library>." and **Take another photo**.

Phone details:

- **Photo preparation.** The page decodes the photo, applies its orientation, scales it so the long side is at most 3072 px (what the label reader uses), and re-encodes it as JPEG at quality 0.9. That covers HEIC and photos over 12 MiB. Inventory photos are stored at 2048 px, so nothing is lost.
- **The token** arrives in the URL fragment (`http://<address>:47822/#t=<token>`), which the browser never sends to the server. The page moves it to `localStorage` and removes it from the address bar. A 401 clears it and shows screen 1 with "This phone is no longer connected."
- **Surviving a lock or reload.** The photo in progress and the cards are kept in IndexedDB until they are saved or discarded. On reload the page restores the review; if a read was running, it posts the same photo again and gets the kept result (see above).
- **When the Mac cannot be reached:** "Cannot reach Corydoras. Check that the Mac is awake, on the same Wi-Fi, and that Phone is on in Settings." The cards stay; **Save** can be pressed again.
- Mobile basics: inputs at 16 px or more (no zoom on focus), `100dvh` layout, safe-area padding for the notch and the home bar, no tap highlight flash, `autocorrect="off"` and `spellcheck="false"` on part numbers and codes, numeric keypad for counts.

### Settings › Phone (`PhonePanel.tsx`)

Shown when the inventory module is on, next to the inventory switch.

- **Let my phone add inventory items over Wi-Fi** (switch) and the port.
- When on: a QR code for `http://<Mac's Wi-Fi IPv4 address>:<port>/#t=<token>`, the same link as text with a Copy button, and the `.local` form (`http://<LocalHostName>.local:<port>`) as text, since some phones do not resolve `.local` names. The address is worked out when the panel opens (the address of the interface that holds the default route), with a **Refresh** button; with no network, it says so.
- **New token** with the inline two-step confirm (`useConfirmRow`): "Phones that scanned the old code are disconnected."
- The text says: the page is reachable from any device on the same network as this Mac while this is on, wherever the Mac is; only a phone that scanned this code can read labels or save; the connection is not encrypted, so use it on a network you trust; each read uses the analysis provider and counts toward the monthly budget, like a read on the Mac.
- The QR code is made in Rust with the `qrcode` crate (SVG output, no image features). This is a new dependency.

### On the Mac when the phone saves

The server emits `library-changed` and a `phone-added` event with the count and the library name. The app shows a toast: "3 items added from your phone." `WorkshopInventory` reloads its list when `library-changed` arrives. That reload changes only the list; an open editor or review keeps its state.

### Serving the page

`npm run build` builds both entries into `dist/`, which Tauri embeds in the app. The server reads `phone.html` and `assets/*` through `app.asset_resolver()`. The builder checks first that this returns the phone entry in a `tauri build --debug` bundle, and which CSP header comes with it; if it does not, the built phone files are embedded in the binary another way. Under `tauri dev` the assets are not embedded; there the page may say "Build the app to use the phone page."

## Testing

- **Rust** (`cargo test -p corydoras`): 401 before the body is read (a request with a large `Content-Length` and no token is answered without reading the body); 413 for an oversize body; 411 without `Content-Length`; module off; library moving; the second request for the same photo waits for or reuses the first read (with a stand-in reader, no provider); a save emits `library-changed`; the bridge still answers as before.
- **Node** (`tests/*.test.mjs`): the phone's request framing and the stored-review logic, where they are plain functions.
- **Live check** on a library copy with the DevCheck build (own identifier, `CORYDORAS_CONFIG_DIR`, a different phone port), following the existing live-check recipe. Open the **LAN address** (not `localhost`, which counts as a secure context and would hide HTTP-only problems) in iOS Simulator Safari, which shares the Mac's network, and in WebKit at phone size. Without provider keys a read fails with the provider message; use **Add an item by hand** to save, and check the item and its photo in the Mac's inventory view and the toast.
- **Only the designer can check:** a real phone camera over real Wi-Fi (what file format the phone hands over), and a paid label read from the phone. A paid read happens only with the designer's OK.

### Specs on the phone

Decided by the designer on 2026-11-19: each card lists its specs, one row per spec, with the value, unit and tolerance and the source in words, using the Mac's wording (`sourceLabel` in `WorkshopSpecsTable.tsx`: "Printed", "Printed, role read from notation", "Standard, not measured", "Entered"). Each row has a **Remove** button. Values cannot be edited on the phone; that stays in the Mac's item editor. Removing a row updates the Dimensions line (`withSpecs`, `removeSpec`). The phone never adds a spec.

Why: it keeps the phone screen short, and a wrong value is worse than a blank, so removing one must be possible where the photo is taken.

Rejected: full editing on the phone (the Mac's table restacked) and read-only specs.

## Changed after review (2026-11-19)

A whole-branch review found no critical issues. These changes followed it; the designer has not yet reviewed them.

- **The `.local` link carries the token** (`http://<LocalHostName>.local:<port>/#t=<token>`) and has a Copy button. Without the token it could not connect a phone. Because the `.local` origin does not change with the Wi-Fi address, a phone connected that way stays connected after the router gives the Mac a new address. Some phones cannot open `.local` addresses.
- **Saves are idempotent.** The page makes a batch id for each photo, keeps it with the photo, and sends it as `X-Batch-Id`. The Mac keeps each save's answer for 24 hours, so a save repeated after a lost answer adds nothing and gets the first answer.
- **A read the Mac answered with an error is not read again by itself.** The page keeps that error with the photo, so a reload shows it with **Read the label again**. Only a read that never reached the Mac (offline, or cancelled by a reload) is asked again on reload. Limit: if the phone locks or reloads during a read that then fails at the provider, the page never sees that error, so the reload reads again.
- **Connections are limited.** At most 16 connections are answered at once; more are closed. The whole request head must arrive within 10 seconds. An accept error pauses for 100 ms. The bridge also gets the head deadline and the accept-error pause.
- **Settings › Phone shows while the link is on**, even when the inventory module is off, with a note, so the link can always be turned off.
- **A save error shows in the save bar**, next to **Save**.
- Tests were added for a library move during phone requests, and for the bridge still answering as before.

## Left to the builder

- The exact module split for the shared HTTP code and the shared inventory guards.
- How many reads the cache keeps (a few) and how it waits on a running read.
- How the phone files are embedded if `asset_resolver` does not serve them.
- Copy details beyond the sentences above.
