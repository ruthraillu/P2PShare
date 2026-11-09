# P2PShare

P2PShare shares and watches screens over peer-to-peer connections on Windows and Android. Named rooms use a compatible signaling Worker. On first launch, enter the WebSocket URL your group uses, then choose a display name and enter a room name.

## Build

- Windows x64 requires stable Rust, the MSVC C++ tools, CMake, and a Windows SDK. Run `cargo build --locked --release --target x86_64-pc-windows-msvc -p p2pshare-desktop -j 1`. The result is `.build/target/x86_64-pc-windows-msvc/release/P2PShare.exe`. Running the app also requires the compatible FFmpeg runtime described in [the codec runtime notes](vendor/ffmpeg-runtime/README.txt).
- Android ARM64 requires Windows PowerShell, stable Rust, and JDK 17 or newer. Run `pwsh -File tools/setup-android.ps1` and then `pwsh -File build-android.ps1 -Action release`. The result is `releases/android/P2PShare-android-arm64.apk`.

The [Compile P2PShare](.github/workflows/compile.yml) workflow runs on pushes to `main`, version tags, and pull requests. You can also run it from the **Actions** tab. Download the EXE and the test APK from the completed run’s **Artifacts** section. Branch builds use a temporary Android test key. Tagged releases use the pinned Android signing certificate.

[config.example.toml](config.example.toml) shows the three first-run fields. Windows creates its local `config.toml` beside the EXE. Android keeps its config in app storage.

## Windows updates

P2PShare checks this public repository for newer stable releases. Open **Update available** to see the release notes. **Download update** is required to fetch the executable. **Restart and install** applies the verified update and keeps your portable config, profile, data and installed codecs.

Commits, pull requests, and manually started workflow runs only build artifacts. A release is published when you explicitly push a new version tag.

To publish an update, bump the workspace version in `Cargo.toml`, update `Cargo.lock`, and push a matching tag such as `v0.13.1`. The tag-only publishing job publishes the Windows EXE, Android APK, and portable Windows ZIP. It also attaches `P2PShare.exe.sig` for the Windows updater. Release builds require the `P2PSHARE_WINDOWS_SIGNING_KEY` and `P2PSHARE_ANDROID_SIGNING_KEY` Actions secrets. The embedded public key verifies the version, executable size and SHA-256 checksum before installation.

For a fresh Windows install, download and extract `P2PShare-Windows-x64.zip`. Existing Windows installs can replace the EXE and keep the codecs folder. Each release verifies and reuses the codec DLLs from the ZIP pinned in [codec-seed.json](vendor/ffmpeg-runtime/codec-seed.json). The ZIP includes the codec license files and a link to the pinned FFmpeg source. Android users download `P2PShare-android-arm64.apk`. The Android app checks for newer APK releases on startup and offers a link to the release page.
