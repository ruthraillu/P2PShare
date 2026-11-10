# P2PShare

P2PShare shares and watches screens over peer-to-peer connections on Windows and Android. Named rooms use a compatible signaling Worker. On first launch, enter the WebSocket URL your group uses, then choose a display name and enter a room name.

## Build

- Windows x64 requires stable Rust, the MSVC C++ tools, CMake, and a Windows SDK. Run `cargo build --locked --release --target x86_64-pc-windows-msvc -p p2pshare-desktop -j 1`. The result is `.build/target/x86_64-pc-windows-msvc/release/P2PShare.exe`. Running the app also requires the compatible FFmpeg runtime described in [the codec runtime notes](vendor/ffmpeg-runtime/README.txt).
- Android ARM64 requires Windows PowerShell, stable Rust, and JDK 17 or newer. Run `pwsh -File tools/setup-android.ps1` and then `pwsh -File build-android.ps1 -Action release`. The result is `releases/android/P2PShare-android-arm64.apk`.

The [Compile P2PShare](.github/workflows/compile.yml) workflow runs on pushes to `main`, version tags, and pull requests. You can also run it from the **Actions** tab. Download the EXE and the test APK from the completed run’s **Artifacts** section. Branch builds use a temporary Android test key. Tagged releases use the pinned Android signing certificate.

[config.example.toml](config.example.toml) shows the three first-run fields. Windows creates its local `config.toml` beside the EXE. Android keeps its config in app storage.

## Windows updates

P2PShare checks this public repository for newer stable releases. Open **Update available** to see the release notes. Click **Download update** to fetch the signed Windows ZIP, then **Restart and install** to apply it. The updater keeps your config.toml, profiles, and personal data. It removes obsolete package files when they still match the installed file inventory.

For a fresh Windows install, extract P2PShare-Windows-x64.zip. If you are on v0.13.0, install v0.13.1 first to switch to ZIP updates. The v0.13.1 release also includes the signed standalone EXE so the earlier updater can install it. Android users can download P2PShare-android-arm64.apk. The Android app checks for newer APK releases on startup and offers a link to the release page.

Commits, pull requests, and manually started workflow runs build artifacts. To publish, bump the workspace version in Cargo.toml, update Cargo.lock, and push a new matching tag. The publishing job uploads the Windows ZIP, its detached signature, and the Android APK. It uses the P2PSHARE_WINDOWS_SIGNING_KEY and P2PSHARE_ANDROID_SIGNING_KEY Actions secrets.

The packager verifies and reuses the codec DLLs from the ZIP pinned in [codec-seed.json](vendor/ffmpeg-runtime/codec-seed.json) and includes the codec license files and a link to the corresponding FFmpeg source. Files under assets/windows-update/ are packaged into data/assets/ and recorded in the managed file inventory.
