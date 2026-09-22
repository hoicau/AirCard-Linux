> Update: the Airlock/Book source experiment succeeded. See
> final sanitized evidence (local-only report): 1929-byte EPUB
> matched at its Books destination, original content was preserved, and the complete
> 56-entry / 762433-byte snapshot was restored. Earlier findings below record the
> investigation, including the recovered initial collection-loss incident.

# Single-asset research in progress

2026-09-22, paired USB iOS 27.0. Grappa index 0 has reached exact ReadyForSync.
The public wire commands FinishedSyncingMetadata and FileComplete have been tested;
SyncFinished is Type=0/session 1 and has no Params field.

The first synthetic request contained only the test item. The device treated the request
as a complete collection and removed an existing extracted EPUB. The complete pre-test
snapshot allowed exact restoration: 35 files and 3 directories were recreated/restored;
all 56 entries and 762433 bytes matched, and the private backup was deleted. See the
sanitized recovery evidence. The original six-file upstream snapshot would not have
covered this loss. Never stage a single-row request on a nonempty collection.

The revised request copies every original row from Books/Books.plist and
Books/Purchases/Purchases.plist, preserving all fields verbatim. Unknown/uncovered EPUB
or PDF paths, missing/unsafe Persistent IDs and catalog collisions block staging.
Original content is checked again before FileComplete; unknown manifest IDs and requests
to re-download retained books are rejected. This revised request preserved original
content, reached SyncFinished and restored the complete snapshot automatically.

File placement is still under investigation. Staging Books/Sync/<AssetID> and sending
AssetPath=Books/<AssetID> left the original staged EPUB untouched even though the device
sent SyncFinished. The resulting catalog row's Path was <AssetID>. Success must require
content verification at the final location, not merely SyncFinished.

Bounded next hypothesis, not a product guarantee: AssetPath may be relative to the Books
sync/staging directory. Test the same safe synthetic leaf as AssetPath, preserving the
entire collection and snapshot. Only that synthetic identity may be logged, replaced
with a fixed placeholder; no directory escape, protected-system write or raw personal
syslog is permitted. Restore and verify the full pre-test snapshot after the attempt.

The leaf-relative hypothesis was superseded **before transmission** by the pinned
[airlift README](https://github.com/0xjohnnydev/airlift/blob/c684cd41ca0ded2d1ab780c15f6ead05509ce062/README.md#atairlock-path-validation):
the normal source is Media/Airlock/Book/<AssetID>; AssetPath is the destination relative
to Media. The next test stages only the exact synthetic leaf under Airlock/Book and
sends AssetPath=Books/<AssetID>. No traversal components or symlinks are used. Parent
directories are removed only if this transaction created them and they remain empty.
