use serde::{Deserialize, Serialize};

use crate::service::Service;

/// Audio format codes for nugs.net.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormatCode {
    Alac = 1,
    Flac = 2,
    Mqa = 3,
    Ra360 = 4,
    Aac = 5,
}

impl FormatCode {
    #[allow(dead_code)] // Used in tests + potential future use
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::Alac),
            2 => Some(Self::Flac),
            3 => Some(Self::Mqa),
            4 => Some(Self::Ra360),
            5 => Some(Self::Aac),
            _ => None,
        }
    }

    /// Return the service-specific numeric format code used in API requests.
    pub fn code(self, service: Service) -> i64 {
        match service {
            Service::Nugs => match self {
                Self::Alac => 1,
                Self::Flac => 2,
                Self::Mqa => 3,
                Self::Ra360 => 4,
                Self::Aac => 5,
            },
            Service::LivePhish => match self {
                Self::Alac => 2,
                Self::Flac => 4,
                Self::Aac => 3,
                // MQA and 360RA not available on LivePhish — fallback to FLAC
                Self::Mqa | Self::Ra360 => 4,
            },
        }
    }

    /// Returns true if this format is available on the given service.
    #[allow(dead_code)] // Used in future phases for service-aware format filtering
    pub fn available_on(self, service: Service) -> bool {
        match service {
            Service::Nugs => true,
            Service::LivePhish => !matches!(self, Self::Mqa | Self::Ra360),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Alac => "alac",
            Self::Flac => "flac",
            Self::Mqa => "mqa",
            Self::Ra360 => "360",
            Self::Aac => "aac",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Alac => "16-bit / 44.1 kHz ALAC",
            Self::Flac => "16-bit / 44.1 kHz FLAC",
            Self::Mqa => "24-bit / 48 kHz MQA",
            Self::Ra360 => "360 Reality Audio",
            Self::Aac => "150 Kbps AAC",
        }
    }

    /// Get the fallback format when this format is unavailable.
    /// Fallback chain: ALAC→FLAC, FLAC→AAC, MQA→FLAC, 360→MQA
    pub fn fallback(self) -> Option<Self> {
        match self {
            Self::Alac => Some(Self::Flac),
            Self::Flac => Some(Self::Aac),
            Self::Mqa => Some(Self::Flac),
            Self::Ra360 => Some(Self::Mqa),
            Self::Aac => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "alac" => Some(Self::Alac),
            "flac" => Some(Self::Flac),
            "mqa" => Some(Self::Mqa),
            "360" => Some(Self::Ra360),
            "aac" => Some(Self::Aac),
            _ => None,
        }
    }
}

/// Audio quality/format info derived from a stream URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Quality {
    pub code: &'static str,
    pub specs: &'static str,
    pub extension: &'static str,
}

impl Quality {
    /// Derive Quality from a FormatCode (used when URL pattern matching fails).
    pub fn from_format_code(fc: FormatCode) -> Self {
        match fc {
            FormatCode::Alac => Quality {
                code: "alac",
                specs: "ALAC",
                extension: ".m4a",
            },
            FormatCode::Flac => Quality {
                code: "flac",
                specs: "FLAC",
                extension: ".flac",
            },
            FormatCode::Mqa => Quality {
                code: "mqa",
                specs: "MQA",
                extension: ".flac",
            },
            FormatCode::Ra360 => Quality {
                code: "360",
                specs: "360 Reality Audio",
                extension: ".mp4",
            },
            FormatCode::Aac => Quality {
                code: "aac",
                specs: "AAC",
                extension: ".m4a",
            },
        }
    }

    /// Detect audio quality from a stream URL by matching known patterns.
    /// Order matters: specific patterns before loose matches.
    pub fn from_stream_url(url: &str) -> Option<Self> {
        // Specific patterns (must come first)
        let patterns: &[(&str, Quality)] = &[
            (
                ".alac16/",
                Quality {
                    code: "alac",
                    specs: "16-bit / 44.1 kHz ALAC",
                    extension: ".m4a",
                },
            ),
            (
                ".flac16/",
                Quality {
                    code: "flac",
                    specs: "16-bit / 44.1 kHz FLAC",
                    extension: ".flac",
                },
            ),
            (
                ".mqa24/",
                Quality {
                    code: "mqa",
                    specs: "24-bit / 48 kHz MQA",
                    extension: ".flac",
                },
            ),
            (
                ".s360/",
                Quality {
                    code: "360",
                    specs: "360 Reality Audio",
                    extension: ".mp4",
                },
            ),
            (
                ".aac150/",
                Quality {
                    code: "aac",
                    specs: "150 Kbps AAC",
                    extension: ".m4a",
                },
            ),
            // Loose matches (must come after specific patterns)
            (
                ".flac?",
                Quality {
                    code: "flac",
                    specs: "FLAC",
                    extension: ".flac",
                },
            ),
            (
                ".m4a?",
                Quality {
                    code: "aac",
                    specs: "AAC",
                    extension: ".m4a",
                },
            ),
            (
                ".m3u8?",
                Quality {
                    code: "hls",
                    specs: "HLS",
                    extension: ".m4a",
                },
            ),
        ];

        for (pattern, quality) in patterns {
            if url.contains(pattern) {
                return Some(quality.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Service;

    #[test]
    fn test_format_code_values() {
        assert_eq!(FormatCode::Alac.code(Service::Nugs), 1);
        assert_eq!(FormatCode::Flac.code(Service::Nugs), 2);
        assert_eq!(FormatCode::Mqa.code(Service::Nugs), 3);
        assert_eq!(FormatCode::Ra360.code(Service::Nugs), 4);
        assert_eq!(FormatCode::Aac.code(Service::Nugs), 5);
    }

    #[test]
    fn test_format_code_values_livephish() {
        assert_eq!(FormatCode::Alac.code(Service::LivePhish), 2);
        assert_eq!(FormatCode::Flac.code(Service::LivePhish), 4);
        assert_eq!(FormatCode::Aac.code(Service::LivePhish), 3);
        // MQA and 360RA fall back to FLAC code on LivePhish
        assert_eq!(FormatCode::Mqa.code(Service::LivePhish), 4);
        assert_eq!(FormatCode::Ra360.code(Service::LivePhish), 4);
    }

    #[test]
    fn test_available_on() {
        assert!(FormatCode::Alac.available_on(Service::Nugs));
        assert!(FormatCode::Mqa.available_on(Service::Nugs));
        assert!(FormatCode::Ra360.available_on(Service::Nugs));
        assert!(FormatCode::Alac.available_on(Service::LivePhish));
        assert!(FormatCode::Flac.available_on(Service::LivePhish));
        assert!(FormatCode::Aac.available_on(Service::LivePhish));
        assert!(!FormatCode::Mqa.available_on(Service::LivePhish));
        assert!(!FormatCode::Ra360.available_on(Service::LivePhish));
    }

    #[test]
    fn test_hls_not_a_format_code() {
        assert!(FormatCode::from_name("hls").is_none());
        assert!(FormatCode::from_code(6).is_none());
    }

    #[test]
    fn test_format_fallback_chain_via_enum() {
        assert_eq!(FormatCode::Alac.fallback(), Some(FormatCode::Flac));
        assert_eq!(FormatCode::Flac.fallback(), Some(FormatCode::Aac));
        assert_eq!(FormatCode::Mqa.fallback(), Some(FormatCode::Flac));
        assert_eq!(FormatCode::Ra360.fallback(), Some(FormatCode::Mqa));
    }

    #[test]
    fn test_from_stream_url_alac() {
        let q = Quality::from_stream_url("https://example.com/path/.alac16/track.m4a").unwrap();
        assert_eq!(q.code, "alac");
        assert_eq!(q.extension, ".m4a");
    }

    #[test]
    fn test_from_stream_url_flac() {
        let q = Quality::from_stream_url("https://example.com/path/.flac16/track.flac").unwrap();
        assert_eq!(q.code, "flac");
        assert_eq!(q.extension, ".flac");
    }

    #[test]
    fn test_from_stream_url_aac() {
        let q = Quality::from_stream_url("https://example.com/path/.aac150/track.m4a").unwrap();
        assert_eq!(q.code, "aac");
        assert_eq!(q.extension, ".m4a");
    }

    #[test]
    fn test_from_stream_url_mqa() {
        let q = Quality::from_stream_url("https://example.com/path/.mqa24/track.flac").unwrap();
        assert_eq!(q.code, "mqa");
        assert_eq!(q.specs, "24-bit / 48 kHz MQA");
        assert_eq!(q.extension, ".flac");
    }

    #[test]
    fn test_from_stream_url_360() {
        let q = Quality::from_stream_url("https://example.com/path/.s360/track.mp4").unwrap();
        assert_eq!(q.code, "360");
        assert_eq!(q.specs, "360 Reality Audio");
        assert_eq!(q.extension, ".mp4");
    }

    #[test]
    fn test_from_stream_url_hls() {
        let q = Quality::from_stream_url("https://example.com/path/.m3u8?token=abc").unwrap();
        assert_eq!(q.code, "hls");
        assert_eq!(q.specs, "HLS");
    }

    #[test]
    fn test_from_stream_url_flac_loose() {
        let q = Quality::from_stream_url("https://example.com/path/track.flac?token=abc").unwrap();
        assert_eq!(q.code, "flac");
    }

    #[test]
    fn test_from_stream_url_m4a_loose() {
        let q = Quality::from_stream_url("https://example.com/path/track.m4a?token=abc").unwrap();
        assert_eq!(q.code, "aac");
    }

    #[test]
    fn test_from_stream_url_unknown_returns_none() {
        let q = Quality::from_stream_url("https://example.com/path/unknown/track.mp3");
        assert!(q.is_none());
    }

    #[test]
    fn test_format_code_from_name() {
        assert_eq!(FormatCode::from_name("flac"), Some(FormatCode::Flac));
        assert_eq!(FormatCode::from_name("alac"), Some(FormatCode::Alac));
        assert_eq!(FormatCode::from_name("unknown"), None);
    }

    #[test]
    fn test_format_code_fallback() {
        assert_eq!(FormatCode::Alac.fallback(), Some(FormatCode::Flac));
        assert_eq!(FormatCode::Flac.fallback(), Some(FormatCode::Aac));
        assert_eq!(FormatCode::Aac.fallback(), None);
    }
}
