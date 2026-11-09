# P2PShare downstream patch

Upstream: winit 0.30.13, copied from the project's cached crates.io dependency.
Only src/platform_impl/android/mod.rs behavior is patched:
- Start: mark the event loop running when a native window exists.
- Pause: keep drawing a live window (PiP / visible multi-window).
- Stop: stop drawing, retaining the existing TerminateWindow suspension path.

This follows Android's visible-but-paused video lifecycle. See
../../docs/android-ui-pip.md. Windows platform code is unchanged.
