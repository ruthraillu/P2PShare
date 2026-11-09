P2PShare Windows codec runtime

Keep all four DLLs in the codecs folder beside P2PShare.exe.
The app exposes only AVC/H.264 and HEVC/H.265.
This is a GPL-enabled FFmpeg shared build with x264/x265 software encoders and NVENC/AMF/QSV backends.
GPU drivers are still supplied by Windows/the hardware vendor.
See build-info.json for pinned source and checksums and FFmpeg-COPYING.GPLv3.txt for the license.
FFmpeg source for these DLLs is available in codecs/source/ in the original v0.13.0 portable ZIP:
https://github.com/ruthraillu/P2PShare/releases/tag/v0.13.0
Pinned upstream FFmpeg source:
https://github.com/FFmpeg/FFmpeg/tree/b139ba11d8e8032cf7e88154b11df19d7ce6af32
Full build configuration is recorded in ffmpeg-build-configuration.txt.
