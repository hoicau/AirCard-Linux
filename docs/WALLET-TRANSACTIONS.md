# Wallet transactions

The application can modify only the selected card's three background resource names and
three generated display-cache names (`FrontFace`, `PlaceHolder`, `Preview`) in `.cache`
and `.pkcache` directories. It cannot select arbitrary protected paths. Imported image names
never become device paths. Lock-screen/TelephonyUI customization is outside this product.

## Apply

Each operation binds a private journal directory to a device fingerprint and card identifier.
A process-wide advisory lock serializes the native Books channel across local app instances.
For each artwork/cache part, the journal stores a stable, bounded original Books snapshot,
a generated transaction UUID, target allowlist and checksummed recovery state. Files use
mode 0600; directories use mode 0700. Atomic replacements fsync the file and parent.

StreamingZip stages payloads and a generated link. ATC moves known originals into a normal
Media staging directory, where AFC can read them. Those bytes are saved durably before
replacement. Existing background files are installed, exported for exact readback, then
returned. Cache leaves are moved out so Wallet regenerates them. Books metadata is restored
between phases; unexpected user content changes stop the operation.

A standalone card backup contains original artwork/cache bytes, expected applied artwork
and the original device binding. It does not contain the Books snapshot. The backup becomes
durable before the transaction enters `Committed`. Cleanup then removes only generated
staging roots, never traversing symlinks. A completed transaction directory is deleted.

## Restore and interruption

Restore reads a validated card backup and takes a *fresh* Books snapshot. It never writes
an old saved book library from the artwork backup. It stages original bytes, restores them,
exports them for byte verification, then returns them. Generated caches are cleared before
cache preimages are restored.

A `Running` transaction recovers original bytes from either the local journal or owned
Media original slots, then restores and verifies them. Fresh numbered rollback directories
separate interrupted move attempts. Unexpected artwork bytes are preserved and reported as
a conflict. Previously moved conflicting bytes are returned before another restore attempt.
Recovery attempts are bounded; retain the journal if a limit or conflict is reported.

A `Committed` transaction has finished the requested result and saved its standalone backup
(where applicable). Recovery finishes cleanup without rolling back that result. `Cleaned`
means all device cleanup finished; local journal deletion can be retried. Known partial
atomic-write temporaries are recognized and removed only after recovery completes.

Disconnects preserve journals. Cancellation allows cleanup before the CLI watchdog ends
stalled native operations. The GUI waits for the CLI completion event and does not infer
success solely from process exit code. The selected device, card, backup and preview digest
are fixed when the user confirms the operation.

## Verification boundary

The device has no supported atomic transaction for the entire operation. Card files are
briefly moved out during backup/readback, so Wallet and Books must stay closed during writes.
A successful ATC `SyncFinished` is insufficient on its own: movement and file bytes are
checked separately. Actual Wallet rendering and post-restore appearance require owner
acceptance and are recorded separately in `STATUS.md`.
