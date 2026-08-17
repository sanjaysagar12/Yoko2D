use std::path::PathBuf; // the path type used throughout this module
use std::sync::mpsc::{channel, Receiver, Sender}; // the channel spawn_watcher hands events back through
use std::time::Duration; // the debounce window type

use notify::RecursiveMode; // the watch-mode argument watch() needs; the Watcher trait itself doesn't need importing to call methods on a `dyn Watcher`
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, DebouncedEventKind, Debouncer}; // the debouncer this module wraps, plus its event-kind enum
use thiserror::Error; // brings in the Error derive macro used below

/// One debounced "this file changed" notification.
#[derive(Debug, Clone)] // Debug: printable in test failures; Clone: cheap, and callers may want to keep a copy around
pub struct WatchEvent {
    pub path: PathBuf, // the path that changed, matching the path spawn_watcher was called with
}

/// Everything that can go wrong setting up a filesystem watcher.
#[derive(Debug, Error)] // Debug: printable in test failures; Error: implements std::error::Error via thiserror
pub enum WatchError {
    /// The underlying `notify`/`notify-debouncer-mini` setup failed (e.g.
    /// the path's parent directory doesn't exist, or the OS refused to
    /// register the watch).
    ///
    /// The underlying error is flattened into a `String` here rather than
    /// wrapped with `#[from]`/kept as a distinct variant per failure mode:
    /// setup failures are rare, one-shot, and calling code has no real use
    /// for pattern-matching on *which* specific notify-internal error
    /// occurred — it only needs to know setup didn't succeed and have a
    /// readable message to show/log.
    #[error("failed to set up file watcher: {0}")]
    SetupFailed(String), // a human-readable description of what went wrong
}

/// Keeps the underlying debouncer (and the OS-level watch it holds) alive.
///
/// `notify`/`notify-debouncer-mini` have no separate "start"/"stop" API —
/// watching lasts exactly as long as the `Debouncer` value is alive, and
/// stops the instant it's dropped. This is a well-known footgun: if
/// `spawn_watcher`'s caller discards the returned handle (e.g.
/// `let _ = spawn_watcher(...)`, or lets it go out of scope early), the
/// watch is torn down immediately and no events will ever arrive — with no
/// error, since nothing actually failed. Holding on to this handle for as
/// long as watching should continue is the caller's responsibility.
pub struct WatcherHandle {
    _debouncer: Debouncer<notify::RecommendedWatcher>, // held only for its Drop impl; the field itself is never read
}

/// Spawns a debounced filesystem watcher on `path`, batching rapid changes
/// within `debounce` into a single [`WatchEvent`] per settle.
///
/// Returns the [`WatcherHandle`] that must be kept alive for watching to
/// continue, plus the [`Receiver`] side of the channel events arrive on.
pub fn spawn_watcher(
    path: PathBuf,      // the exact file to watch for changes
    debounce: Duration, // how long to wait for writes to settle before emitting one event
) -> Result<(WatcherHandle, Receiver<WatchEvent>), WatchError> {
    let (sender, receiver): (Sender<WatchEvent>, Receiver<WatchEvent>) = channel(); // the channel this function's caller reads events from
    let watched_path = path.clone(); // cloned so the debouncer callback (which outlives this function) can own its own copy

    let mut debouncer = new_debouncer(debounce, move |result: DebounceEventResult| {
        // invoked on a background thread by notify-debouncer-mini whenever a debounced batch settles
        let events = match result {
            Ok(events) => events, // a successful batch of debounced filesystem events
            Err(_) => return, // a watch-level error (e.g. the OS watch itself failed mid-run): nothing actionable here, so drop it
        };
        for event in events {
            // a debounced batch can in principle name more than one path, even though we only watch one file
            if event.path != watched_path {
                // not our file: ignore (defensive filtering, in case notify ever reports a related sibling path)
                continue; // move on to the next event in this batch
            }
            if event.kind != DebouncedEventKind::Any {
                // `AnyContinuous` means "writes are still ongoing, not yet settled" — notify-debouncer-mini
                // emits this whenever a burst of writes for one path spans past the debounce window while
                // still arriving, ahead of the final `Any` it emits once writes actually stop. Since even a
                // handful of writes only microseconds apart can straddle that boundary (its settle deadline
                // is anchored to the *first* write in a burst, not the *last*), a burst that should collapse
                // to one logical "file changed" notification can otherwise produce an extra continuation
                // event first. Forwarding only the terminal `Any` kind is what actually collapses a burst
                // into exactly one `WatchEvent`, not merely a large debounce duration.
                continue; // not a final settle: don't forward it as a WatchEvent
            }
            let _ = sender.send(WatchEvent {
                path: watched_path.clone(),
            }); // ignore send errors: a dropped receiver just means no one is listening anymore, not a bug
        }
    })
    .map_err(|err| WatchError::SetupFailed(err.to_string()))?; // debouncer construction failed: report it, never panic

    // Watching the exact file path (NonRecursive, since there's no
    // directory tree involved) rather than its parent directory: this
    // crate's writes are plain in-place `fs::write` calls (truncate +
    // write), not the write-new-file-then-rename-over-original pattern
    // some editors use, so watching the file's own path directly is
    // sufficient to reliably observe content changes here. (A future
    // phase that needs to tolerate arbitrary external editors doing
    // atomic renames would likely need to watch the parent directory
    // instead, since a rename can leave a direct file-path watch stale.)
    debouncer
        .watcher()
        .watch(&path, RecursiveMode::NonRecursive) // register the OS-level watch on this single file
        .map_err(|err| WatchError::SetupFailed(err.to_string()))?; // registering the watch failed: report it, never panic

    let handle = WatcherHandle {
        _debouncer: debouncer,
    }; // wrap the debouncer so dropping the handle is what stops watching
    Ok((handle, receiver)) // hand back the handle to keep alive, and the receiver to read events from
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn detects_a_single_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watched.txt");
        std::fs::write(&path, "initial").unwrap();

        let (_handle, receiver) = spawn_watcher(path.clone(), Duration::from_millis(100)).unwrap();

        std::fs::write(&path, "updated").unwrap();

        let event = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event.path, path);
    }

    #[test]
    fn rapid_writes_within_debounce_window_collapse_to_one_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("watched.txt");
        std::fs::write(&path, "initial").unwrap();

        // 200ms is plenty here: what originally made this test flaky
        // wasn't debounce duration at all, but notify-debouncer-mini
        // emitting an interim `AnyContinuous` event ahead of the final
        // `Any` for *any* burst of more than one write to the same path
        // (its settle deadline is anchored to the burst's first write, not
        // its last, so a second internal tick — and thus a second event —
        // is essentially inevitable once more than one write lands within
        // a cycle, regardless of how large the debounce window is). Fixed
        // in `spawn_watcher` itself by only forwarding `DebouncedEventKind::Any`
        // (see the comment there), so this test now genuinely collapses to 1.
        let debounce = Duration::from_millis(200);
        let (_handle, receiver) = spawn_watcher(path.clone(), debounce).unwrap();

        // Five rapid writes, well inside the debounce window, with no sleep between them.
        for i in 0..5 {
            std::fs::write(&path, format!("update {i}")).unwrap();
        }

        // Wait past the debounce window before draining. notify-debouncer-mini's
        // default batch_mode can delay a settle by up to 2x the configured
        // timeout, so wait for 2x debounce plus a margin, not just 1x.
        std::thread::sleep(debounce * 2 + Duration::from_millis(300));

        let mut count = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match receiver.recv_timeout(Duration::from_millis(300)) {
                Ok(_) => count += 1,
                Err(_) => break, // timed out waiting for another event: the batch is exhausted
            }
        }

        assert_eq!(count, 1);
    }

    #[test]
    fn setup_fails_gracefully_for_a_path_whose_parent_does_not_exist() {
        let path = PathBuf::from("/definitely/does/not/exist/file.json");
        let result = spawn_watcher(path, Duration::from_millis(100));
        assert!(matches!(result, Err(WatchError::SetupFailed(_))));
    }
}
