# Development Runbook

This runbook documents the current local workflow for stable WebGL iteration.

## Recommended Local Server

Use one Trunk dev server, managed by `tmux`, on a fixed local port.

Loopback workflow (desktop only):

```bash
tmux new-session -d -s bvb-dev \
  'cd /Users/michaelgrilo/beyond-vs-below && env NO_COLOR=false TRUNK_NO_COLOR=true trunk serve --address 127.0.0.1 --port 4173 --no-autoreload'
```

Open:

```bash
open http://127.0.0.1:4173
```

Stop:

```bash
tmux kill-session -t bvb-dev
```

Inspect recent server output:

```bash
tmux capture-pane -pt bvb-dev:0 -S -200
```

HTTPS LAN workflow (required for phone offline service-worker tests):

```bash
cd /Users/michaelgrilo/beyond-vs-below
./scripts/dev_https_start.sh
```

The script:

- generates `.tls/bvb-dev-root-ca.crt` (one-time root CA)
- generates `.tls/bvb-lan.crt` for your current LAN IP
- starts Trunk in `tmux` session `bvb-https` with TLS + `wss`

Stop:

```bash
./scripts/dev_https_stop.sh
```

Trust setup for phone (one-time per device):

1. Copy `/Users/michaelgrilo/beyond-vs-below/.tls/bvb-dev-root-ca.crt` to the phone.
2. Install the certificate profile on the phone.
3. iOS only: enable full trust for that root CA in:
   - `Settings -> General -> About -> Certificate Trust Settings`
4. Re-open `https://<lan-ip>:4173` and confirm there is no TLS warning page.

Environment note:

- Some shells export `NO_COLOR=1`, which Trunk can parse incorrectly in this setup.
- Use explicit boolean env values when launching Trunk (`NO_COLOR=false` and `TRUNK_NO_COLOR=true` as shown above).

## Flicker/Reset Root Cause and Fix

Observed root cause:

- Multiple `trunk serve` processes running at the same time.
- Both instances writing to the same `dist` staging/final directories.
- Repeated Trunk staging errors and rebuild churn, which looked like render flicker and scene resets.

Fix:

- Ensure only one `trunk serve` is active for this repository.
- Use `--no-autoreload` during manual visual testing.

Useful check:

```bash
lsof -nP -iTCP -sTCP:LISTEN | rg '4173|4175|trunk'
```

If multiple Trunk listeners appear, stop extras before testing.

## Known Symptoms of Dev-Server Churn

- Title screen appears/disappears or flickers.
- Diagnostics tool icon flickers.
- Tap/click to enter level appears to "refresh" back to title.
- Intermittent dist staging errors in Trunk logs.

## Current Runtime Behavior

- Title screen loads first.
- Tap/click on the canvas starts the game transition.
- Level map is loaded from `assets/levels/mall_parking_lot.wasm`.
- Diagnostics panel is toggleable from the top-right tools button.
- Diagnostics panel is constrained to the mobile 9:16 frame overlay.

## Service Worker Notes (Local Dev)

- Service worker registration is enabled by default on `localhost` and `127.0.0.1`.
- Registration now prunes stale service workers that are not on `sw_bootstrap_sync.js`.
- Runtime route caching is disabled on localhost to prevent reload/flicker loops during Trunk iteration.
- Both bootstrap paths are served for compatibility: `sw_bootstrap_sync.js` and `sw_bootstrap.js`.
- Use `?nosw=1` to run without service worker caching.
- Phone/LAN offline validation must use `https://<lan-ip>:4173` (secure context).
- `http://<lan-ip>:4173` is not sufficient for reliable service-worker offline behavior on phone browsers.

## Offline Incident Notes (2026-02-11)

What went wrong:

- Browser served stale app shell/service-worker files from earlier bootstrap versions.
- Local offline reload showed `ERR_CONNECTION_REFUSED` when the service worker was not installed/controlling that tab.
- Earlier service-worker bootstrap variants triggered Chromium lifecycle warnings:
  - install/activate/fetch handlers not registered at initial script evaluation.
  - top-level await invalid in service workers.
- DevTools console included noisy local-dev errors after server stop:
  - Trunk websocket reconnect failures.
  - `favicon.ico` 500.
  - preload integrity warning for unsupported preload destination.

Changes made:

- `Trunk.toml`
  - Set `[build] no_sri = true`.
  - Set `[serve] no_autoreload = true`.
- `index.html`
  - Added explicit favicon link:
    - `/assets/title_screen/title_screen.png`
  - Kept SW bootstrap files copied by Trunk:
    - `sw_bootstrap_sync.js`
    - `sw_bootstrap.js`
- `scripts/build_sw.sh`
  - Generates synchronous SW bootstrap with `initSync`.
  - Embeds SW wasm as single-line base64.
  - Registers placeholder install/activate/fetch listeners at initial script evaluation.
  - Writes bootstrap files atomically (`.tmp` then `mv`).
- `src/lib.rs`
  - Bumped SW bootstrap URL cache-bust to:
    - `/sw_bootstrap_sync.js?v=sync-init-5`
  - Keeps stale SW registration pruning logic.
- `sw/src/lib.rs`
  - Bumped static cache to `bvb-static-v3`.
  - Added purge of old cache names (`bvb-static-v1`, `bvb-static-v2`) during activate.

## Deterministic Offline Test Flow

1. Start HTTPS LAN server:
   - `./scripts/dev_https_start.sh`
2. Open `https://<lan-ip>:4173` on the phone.
3. In diagnostics, confirm:
   - `sw_controlled: true`
   - `sw_script` contains `sw_bootstrap_sync.js`
4. In DevTools Application tab, confirm service worker status is `activated and running`.
5. Stop server:
   - `./scripts/dev_https_stop.sh`
6. Reload the same tab.
7. Expected result:
   - App still renders from cache offline.
   - If page instead shows `ERR_CONNECTION_REFUSED`, the tab is not SW-controlled yet.

## Stability Check (Optional)

A quick non-interactive stability check is to take two headless screenshots and compare checksums.
Identical checksums indicate stable visual output for the sampled frame.
