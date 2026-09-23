# AirCard-Linux v0.1.1

This release makes first-time Wallet artwork setup easier with automatic card detection
and sync token setup.

## Improvements

- **Automatic card detection:** Opening **Apply & restore** with a paired iPhone starts
  detection. Wait for the prompt, then open Wallet and tap the intended card. A single
  detected identifier is filled in automatically; multiple candidates remain a choice.
  Detection stops shortly after a match or after 60 seconds.
- **Automatic sync token setup:** After a card is selected, AirCard downloads the token
  from a pinned public source, verifies its SHA-256 and format, and saves it with private
  permissions. Future launches reuse the cached token offline. Manual file selection
  and a setup/retry button remain available for Apply, Restore and Recover.
- **Clearer troubleshooting:** The interface shows when Wallet detection is ready and
  provides retry guidance for missing identifiers, missing logs and connection failures.
  Token download failures include recovery steps.
- **More complete identifier detection:** Fixes matching of padded and unpadded SHA-1
  and SHA-256 identifiers, accepts valid URL-safe Base64 hashes, and collects multiple
  identifiers from the same log line.
- **CLI additions:** `aircard setup-token [--output PATH]` sets up a token without a
  connected iPhone. `aircard scan --until-match` ends after detecting candidates.

## Requirements and limits

- First-time automatic token setup needs internet access to GitHub, `curl` and
  `ca-certificates`. Existing valid token files continue to work.
- You still open Wallet and tap the intended card on your iPhone, then review and
  confirm before artwork is changed. Detection relies on identifiers exposed in Wallet
  logs; some cards or iOS versions may not expose them. Background Wallet activity can
  also produce identifiers, so verify the intended target.
- Downloading a token does not guarantee iOS compatibility. Existing hardware acceptance
  covers USB artwork application, restore and interrupted recovery on one iOS 27.0 iPhone.
  The new automatic flow has not yet been verified on a physical iPhone. Wi-Fi and other
  iOS versions remain unverified.

## Validation

Workspace tests, formatting, Clippy and release build; native GUI smoke captures covering
light/dark themes, compact windows, waiting for Wallet and no-card guidance; live pinned
token download, private-file permissions and offline cache reuse.
