//! Backend names are not hardware evidence. Bind every candidate to its vendor.
use p2pshare_media::config::VideoFormat;
pub(crate) fn software_matches(name: &str, codec: VideoFormat) -> bool {
    matches!(
        (name, codec),
        ("libx264", VideoFormat::H264) | ("libx265", VideoFormat::Hevc)
    )
}
pub(crate) fn encoder_matches(name: &str, codec: VideoFormat, vendor: u32) -> bool {
    let prefix = match codec {
        VideoFormat::H264 => "h264_",
        VideoFormat::Hevc => "hevc_",
    };
    match name.strip_prefix(prefix) {
        Some("nvenc") => vendor == 0x10de,
        Some("amf") => vendor == 0x1002,
        Some("qsv") => vendor == 0x8086,
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_cached_names_cannot_turn_nvidia_or_intel_into_amf() {
        for codec in [VideoFormat::H264, VideoFormat::Hevc] {
            let prefix = if codec == VideoFormat::H264 {
                "h264"
            } else {
                "hevc"
            };
            for (family, vendor) in [("nvenc", 0x10de), ("amf", 0x1002), ("qsv", 0x8086)] {
                for actual in [0x10de, 0x1002, 0x8086, 0x1414] {
                    assert_eq!(
                        encoder_matches(&format!("{prefix}_{family}"), codec, actual),
                        vendor == actual
                    );
                }
            }
        }
        assert!(!encoder_matches("h264_amf", VideoFormat::Hevc, 0x1002));
        assert!(!encoder_matches("pretend_amf", VideoFormat::H264, 0x1002));
        assert!(!software_matches("libx264", VideoFormat::Hevc));
    }
}
