# Desktop presentation

`app.rs` draws the room grid, focused view, fullscreen and footer. `dialogs.rs` draws the source picker, quality, onboarding and appearance. `config.rs` translates persisted color tokens into egui styles. `scenes.rs` is synthetic artwork used exclusively when `App.live` is false.

The production adapter is `../desktop-ui/src/design_adapter.rs`. It supplies real participant IDs, sharing state, textures, source IDs and dimensions, and consumes typed `Intent` events. Capture, codecs, audio and signaling remain in their existing crates. The Android UI continues to use `room-ui`.

`App::live` never fabricates participants, frames, thumbnails or latency. Own preview audio controls are disabled because that preview does not replay audio. Mute preferences and focus follow stable participant IDs. Layout controls use stable egui IDs across participant changes.

The standalone prototype uses its own isolated copy at `../../prototypes/desktop-ui/design/`. Changes here affect the production app; changes in that copy affect only the sample. Its offscreen WARP gallery checks actual widgets at normal/compact/200% DPI, demo and live modes. Live tests use generated textures, without screen/audio capture or desktop windows.

Picker thumbnails are handled separately in `../desktop-ui/src/thumbnails.rs`: a background MTA, one WGC source at a time, GPU downscale to 360x202 and bounded two-image transfer. Each picker opening takes one static snapshot of up to 96 sources; tabs and searches reuse those snapshots. The worker releases its GPU device after the batch finishes. Closing or occluding the app cancels unfinished preview work. Protected/minimized/unavailable windows may have no preview. Streaming never depends on thumbnail readiness.

Production configuration is `P2PAPP/config.toml`, with twelve commented `[theme]` colors. Fresh portable installations require a name; `profile_setup_complete = false` can be used to show onboarding again. Existing profiles retain their name. Share settings preserve backend/thread/logging/audio bitrate/signaling options. Playback buffer is viewer-side, 0..1000 ms.

Window placement is isolated in `../desktop-ui/src/window_memory.rs`. `P2PAPP/window-state.toml` records the normal window bounds and maximized state, with debounced writes and a forced save on clean exit. Borderless fullscreen/minimized bounds are excluded. Removing that file resets placement. Test/smoke modes ignore it.

`chrome.rs` draws the 32-pixel custom title bar and resize hit zones using the existing theme. Window actions are typed presentation intents. Native adapters handle minimize, maximize/restore, close, dragging and edge resizing. The title bar stays interactive above modal dialogs and disappears in fullscreen. The window has no standard Windows decorations.

Single-line dialog inputs vertically center text. Share resolution labels use 480p, 720p, 1080p, 1440p and 2160p presets; the capture pipeline keeps the source aspect ratio within the corresponding 16:9 bounds. Stream validation permits up to a 3840x2160 pixel budget.

Native fullscreen transitions restore a maximized window before entering borderless fullscreen, avoiding winit's taskbar-work-area clamp. Exiting fullscreen restores the previous maximized state. WGC sessions request `IsBorderRequired = false` without changing capture or codec settings.

Fullscreen controls are isolated in fullscreen.rs. The toolbar appears only while the pointer is inside the bottom 96 logical pixels of the viewport, with no movement/click timer. Exit fullscreen includes an Esc badge; mute/unmute uses a speaker icon and readable labels. Esc also works while the toolbar is hidden.


Room streams default to JPEG previews. `Watch screen` requests full video/audio; `Stop watching` returns to a cached JPEG and releases live decoder resources. The sharer controls `stream.preview_interval_secs` (10..60, default 10) in the share settings. Preview frames preserve aspect ratio with neither dimension exceeding 512 pixels. They use a direct encrypted data channel, with no Worker media traffic. With no subscribers, the publisher does not run the full video encoder.

Stream tile footers are isolated in `stream_footer.rs`: one 44-pixel row with vertically centered name, resolution/stats and controls. Narrow tiles use smaller control widths and text, with truncated names and hover details rather than a second row.

The footer shows resolution, actual AVC/HEVC format and configured target bitrate. Stream details use type 3 on the existing encrypted peer control channel, and follow encoder fallback. Focus borders blend 75% theme border with 25% accent.

The local tile has Hide preview / Show preview beside Mute. It changes presentation state only, including focused thumbnails and fullscreen. The native adapter skips local preview texture copies while hidden, but still checks capture state; the independent publisher, remote JPEG previews and audio continue. This preference lasts for the current app session.

Rooms without visible live video use a 50 ms state/redraw timer; input events still redraw immediately. Visible own previews and watched remote streams retain the original Present/vsync cadence. Hidden local and JPEG-only rooms do not force display-refresh redraws. Publisher idle polling is 20 ms while no remote viewer is watching, with capture preview polling retained at 10 Hz. Idle encoding is explicit in stream_health, including viewer count, preview visibility and published frame count.
