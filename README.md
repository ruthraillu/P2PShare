# P2PShare

P2PShare shares and watches screens over peer-to-peer connections on Windows and Android. Named rooms use a compatible signaling Worker. On first launch, enter the WebSocket URL your group uses, then choose a display name and enter a room name.

## Build

- Windows x64 requires stable Rust, the MSVC C++ tools, CMake, and a Windows SDK. Run `cargo build --locked --release --target x86_64-pc-windows-msvc -p p2pshare-desktop -j 1`. The result is `.build/target/x86_64-pc-windows-msvc/release/P2PShare.exe`. Running the app also requires the compatible FFmpeg runtime described in [the codec runtime notes](vendor/ffmpeg-runtime/README.txt).
- Android ARM64 requires Windows PowerShell, stable Rust, and JDK 17 or newer. Run `pwsh -File tools/setup-android.ps1` and then `pwsh -File build-android.ps1 -Action release`. The result is `releases/android/P2PShare-android-arm64.apk`.

The [Compile P2PShare](.github/workflows/compile.yml) workflow runs on pushes to `main`, version tags, and pull requests. You can also run it from the **Actions** tab. Download the EXE and the test APK from the completed run’s **Artifacts** section. The APK uses a temporary signing key on GitHub-hosted runners and is intended for fresh test installs.

[config.example.toml](config.example.toml) shows the three first-run fields. Windows creates its local `config.toml` beside the EXE. Android keeps its config in app storage.
