# Native embedded directory storage

With `embedded-sparq`, `EmbeddedSparqClient::open(dir)` opens a saved graph and
keeps its directory locks on the graph's actor thread. Dropping the last client
drains queued jobs and joins that thread before returning. A save into the open
directory compacts it; a save elsewhere writes an independent snapshot.

The graph holds `<dir>.lock` beside its directory when available, and `sparq.lock`
inside it. An inside-only holder can serve a store whose parent is not writable,
but cannot compact by renaming the root directory. Never remove these lock files
while the directory may be in use.

On Unix, child-process creation can briefly retain the graph's lock descriptors
between fork and exec, even after the parent's graph has been dropped.
Close-on-exec only closes them at exec. The embedded opener therefore retries
only `WouldBlock`, with a **250 ms retry budget** and waits of at most **1 ms**
between attempts. Each attempt runs the whole graph open, including both locks.
A holder that remains live is refused when the budget expires; other I/O errors
return immediately. This is a synchronous open and can block its calling thread
for that retry budget, subject to filesystem and scheduler delays. It does not
wait indefinitely for another server to stop.

An inherited descriptor retained beyond the budget can still cause `WouldBlock`;
the bound covers transient contention without promising that every reopen succeeds.

The lower-level `Graph::open` and `DirLock` APIs retain their immediate refusal
on contention. No directory-lock protocol or stored-file format changes.

The descriptor lifetime follows the documented
[Linux `flock` semantics](https://man7.org/linux/man-pages/man2/flock.2.html) and
[macOS `flock` semantics](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html).
