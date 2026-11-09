# P2PShare

P2PShare shares and watches screens over peer-to-peer connections on Windows and Android. Named rooms use a compatible signaling Worker. On first launch, enter the WebSocket URL your group uses, then choose a display name and enter a room name.

## Build

- Windows x64 requires stable Rust, the MSVC C++ tools, CMake, and a Windows SDK. Run `cargo build --locked --release --target x86_64-pc-windows-msvc -p p2pshare-desktop -j 1`. The result is `.build/target/x86_64-pc-windows-msvc/release/P2PShare.exe`. Running the app also requires the compatible FFmpeg runtime described in [the codec runtime notes](vendor/ffmpeg-runtime/README.txt).
- Android ARM64 requires Windows PowerShell, stable Rust, and JDK 17 or newer. Run `pwsh -File tools/setup-android.ps1` and then `pwsh -File build-android.ps1 -Action release`. The result is `releases/android/P2PShare-android-arm64.apk`.

The [Compile P2PShare](.github/workflows/compile.yml) workflow runs on pushes to `main`, version tags, and pull requests. You can also run it from the **Actions** tab. Download the EXE and the test APK from the completed run’s **Artifacts** section. The APK uses a temporary signing key on GitHub-hosted runners and is intended for fresh test installs.

[config.example.toml](config.example.toml) shows the three first-run fields. Windows creates its local `config.toml` beside the EXE. Android keeps its config in app storage.

## Windows updates

P2PShare checks this public repository for newer stable releases. Open **Update available** to see the release notes. **Download update** is required to fetch the executable. **Restart and install** applies the verified update and keeps your portable config, profile, data and installed codecs.

To publish an update, bump the workspace version in `Cargo.toml`, update `Cargo.lock`, and push a matching tag such as `v0.13.1`. The Windows job compiles the EXE and the tag-only publishing job signs and uploads `P2PShare.exe` and `P2PShare.exe.sig`. The publishing job requires the repository’s `P2PSHARE_WINDOWS_SIGNING_KEY` Actions secret. The embedded public key verifies the version, executable size and SHA-256 checksum before installation.

The Windows updater replaces the EXE only. Use the matching FFmpeg DLLs for the initial portable install. Android remains a separately built test artifact.
