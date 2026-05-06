use crate::core::error::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Features(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthMethod {
    None,
    HomeKitTransient,
    FairPlay,
    MfiRequired,
}

impl Features {
    pub const SUPPORTS_VIDEO_V1: u64 = 1 << 0;
    pub const SUPPORTS_PHOTO: u64 = 1 << 1;
    pub const SUPPORTS_SLIDESHOW: u64 = 1 << 5;
    pub const SUPPORTS_SCREEN: u64 = 1 << 7;
    pub const SUPPORTS_AUDIO: u64 = 1 << 9;
    pub const AUDIO_REDUNDANT: u64 = 1 << 11;
    pub const AUTHENTICATION_FAIRPLAY: u64 = 1 << 14;
    pub const METADATA_FEATURES_0: u64 = 1 << 15;
    pub const METADATA_FEATURES_1: u64 = 1 << 16;
    pub const METADATA_FEATURES_2: u64 = 1 << 17;
    pub const AUDIO_FORMATS_0: u64 = 1 << 18;
    pub const AUDIO_FORMATS_1: u64 = 1 << 19;
    pub const AUDIO_FORMATS_2: u64 = 1 << 20;
    pub const AUDIO_FORMATS_3: u64 = 1 << 21;
    pub const AUTHENTICATION_1: u64 = 1 << 23;
    pub const AUTHENTICATION_MFI: u64 = 1 << 26;
    pub const SUPPORTS_LEGACY_PAIRING: u64 = 1 << 27;
    pub const HAS_UNIFIED_ADVERTISER_INFO: u64 = 1 << 30;

    pub const IS_CARPLAY: u64 = 1 << 32;
    pub const SUPPORTS_VIDEO_PLAY_QUEUE: u64 = 1 << 33;
    pub const SUPPORTS_AIRPLAY_FROM_CLOUD: u64 = 1 << 34;
    pub const SUPPORTS_TLS_PSK: u64 = 1 << 35;
    pub const SUPPORTS_UNIFIED_MEDIA_CONTROL: u64 = 1 << 38;
    pub const SUPPORTS_BUFFERED_AUDIO: u64 = 1 << 40;
    pub const SUPPORTS_PTP: u64 = 1 << 41;
    pub const SUPPORTS_SCREEN_MULTI_CODEC: u64 = 1 << 42;
    pub const SUPPORTS_SYSTEM_PAIRING: u64 = 1 << 43;
    pub const IS_AP_VALERIA_SCREEN_SENDER: u64 = 1 << 44;
    pub const SUPPORTS_HOMEKIT_PAIRING: u64 = 1 << 46;
    pub const SUPPORTS_TRANSIENT_PAIRING: u64 = 1 << 48;
    pub const SUPPORTS_VIDEO_V2: u64 = 1 << 49;
    pub const METADATA_FEATURES_3: u64 = 1 << 50;
    pub const SUPPORTS_UNIFIED_PAIR_MFI: u64 = 1 << 51;
    pub const SUPPORTS_SET_PEERS_EXTENDED_MESSAGE: u64 = 1 << 52;
    pub const SUPPORTS_AP_SYNC: u64 = 1 << 54;
    pub const SUPPORTS_WOL_55: u64 = 1 << 55;
    pub const SUPPORTS_WOL_56: u64 = 1 << 56;
    pub const SUPPORTS_HANGDOG_REMOTE_CONTROL: u64 = 1 << 58;
    pub const SUPPORTS_AUDIO_STREAM_CONNECTION_SETUP: u64 = 1 << 59;
    pub const SUPPORTS_AUDIO_MEDIA_DATA_CONTROL: u64 = 1 << 60;
    pub const SUPPORTS_RFC2198_REDUNDANCY: u64 = 1 << 61;
}

impl Features {
    pub fn from_txt_value(s: &str) -> Result<Self, ParseError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ParseError::InvalidFormat(
                "empty features string".to_string(),
            ));
        }

        fn parse_hex(part: &str) -> Result<u64, ParseError> {
            let part = part.trim();
            let hex_str = part
                .strip_prefix("0x")
                .or_else(|| part.strip_prefix("0X"))
                .unwrap_or(part);

            if hex_str.is_empty() {
                return Err(ParseError::InvalidHex("empty hex value".to_string()));
            }

            u64::from_str_radix(hex_str, 16).map_err(|_| ParseError::InvalidHex(part.to_string()))
        }

        let (lower, upper) = match s.split_once(',') {
            Some((lower_str, upper_str)) => {
                let lower = parse_hex(lower_str)?;
                let upper = parse_hex(upper_str)?;
                (lower, upper)
            }
            None => {
                let lower = parse_hex(s)?;
                (lower, 0u64)
            }
        };

        Ok(Self(lower | (upper << 32)))
    }

    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub fn raw(&self) -> u64 {
        self.0
    }

    pub fn to_txt_value(&self) -> String {
        let lower = self.0 & 0xFFFF_FFFF;
        let upper = self.0 >> 32;

        if upper == 0 {
            format!("0x{:X}", lower)
        } else {
            format!("0x{:X},0x{:X}", lower, upper)
        }
    }

    pub fn supports_video_v1(&self) -> bool {
        self.0 & Self::SUPPORTS_VIDEO_V1 != 0
    }

    pub fn supports_photo(&self) -> bool {
        self.0 & Self::SUPPORTS_PHOTO != 0
    }

    pub fn supports_slideshow(&self) -> bool {
        self.0 & Self::SUPPORTS_SLIDESHOW != 0
    }

    pub fn supports_screen(&self) -> bool {
        self.0 & Self::SUPPORTS_SCREEN != 0
    }

    pub fn supports_audio(&self) -> bool {
        self.0 & Self::SUPPORTS_AUDIO != 0
    }

    pub fn supports_redundant_audio(&self) -> bool {
        self.0 & Self::AUDIO_REDUNDANT != 0
    }

    pub fn requires_fairplay(&self) -> bool {
        self.0 & Self::AUTHENTICATION_FAIRPLAY != 0
    }

    pub fn metadata_features(&self) -> u8 {
        let b0 = (self.0 >> 15) & 1;
        let b1 = (self.0 >> 16) & 1;
        let b2 = (self.0 >> 17) & 1;
        let b3 = (self.0 >> 50) & 1;
        (b0 | (b1 << 1) | (b2 << 2) | (b3 << 3)) as u8
    }

    pub fn audio_formats(&self) -> u8 {
        ((self.0 >> 18) & 0xF) as u8
    }

    pub fn has_authentication_1(&self) -> bool {
        self.0 & Self::AUTHENTICATION_1 != 0
    }

    pub fn requires_mfi(&self) -> bool {
        self.0 & Self::AUTHENTICATION_MFI != 0
    }

    pub fn supports_legacy_pairing(&self) -> bool {
        self.0 & Self::SUPPORTS_LEGACY_PAIRING != 0
    }

    pub fn has_unified_advertiser_info(&self) -> bool {
        self.0 & Self::HAS_UNIFIED_ADVERTISER_INFO != 0
    }

    pub fn is_carplay(&self) -> bool {
        self.0 & Self::IS_CARPLAY != 0
    }

    pub fn supports_video_play_queue(&self) -> bool {
        self.0 & Self::SUPPORTS_VIDEO_PLAY_QUEUE != 0
    }

    pub fn supports_airplay_from_cloud(&self) -> bool {
        self.0 & Self::SUPPORTS_AIRPLAY_FROM_CLOUD != 0
    }

    pub fn supports_tls_psk(&self) -> bool {
        self.0 & Self::SUPPORTS_TLS_PSK != 0
    }

    pub fn supports_unified_media_control(&self) -> bool {
        self.0 & Self::SUPPORTS_UNIFIED_MEDIA_CONTROL != 0
    }

    pub fn supports_buffered_audio(&self) -> bool {
        self.0 & Self::SUPPORTS_BUFFERED_AUDIO != 0
    }

    pub fn supports_ptp(&self) -> bool {
        self.0 & Self::SUPPORTS_PTP != 0
    }

    pub fn supports_screen_multi_codec(&self) -> bool {
        self.0 & Self::SUPPORTS_SCREEN_MULTI_CODEC != 0
    }

    pub fn supports_system_pairing(&self) -> bool {
        self.0 & Self::SUPPORTS_SYSTEM_PAIRING != 0
    }

    pub fn is_ap_valeria_screen_sender(&self) -> bool {
        self.0 & Self::IS_AP_VALERIA_SCREEN_SENDER != 0
    }

    pub fn supports_homekit_pairing(&self) -> bool {
        self.0 & Self::SUPPORTS_HOMEKIT_PAIRING != 0
    }

    pub fn supports_transient_pairing(&self) -> bool {
        self.0 & Self::SUPPORTS_TRANSIENT_PAIRING != 0
    }

    pub fn supports_video_v2(&self) -> bool {
        self.0 & Self::SUPPORTS_VIDEO_V2 != 0
    }

    pub fn supports_unified_pair_mfi(&self) -> bool {
        self.0 & Self::SUPPORTS_UNIFIED_PAIR_MFI != 0
    }

    pub fn supports_set_peers_extended_message(&self) -> bool {
        self.0 & Self::SUPPORTS_SET_PEERS_EXTENDED_MESSAGE != 0
    }

    pub fn supports_ap_sync(&self) -> bool {
        self.0 & Self::SUPPORTS_AP_SYNC != 0
    }

    pub fn supports_wol(&self) -> bool {
        self.0 & (Self::SUPPORTS_WOL_55 | Self::SUPPORTS_WOL_56) != 0
    }

    pub fn supports_hangdog_remote_control(&self) -> bool {
        self.0 & Self::SUPPORTS_HANGDOG_REMOTE_CONTROL != 0
    }

    pub fn supports_audio_stream_connection_setup(&self) -> bool {
        self.0 & Self::SUPPORTS_AUDIO_STREAM_CONNECTION_SETUP != 0
    }

    pub fn supports_audio_media_data_control(&self) -> bool {
        self.0 & Self::SUPPORTS_AUDIO_MEDIA_DATA_CONTROL != 0
    }

    pub fn supports_rfc2198_redundancy(&self) -> bool {
        self.0 & Self::SUPPORTS_RFC2198_REDUNDANCY != 0
    }

    pub fn auth_method(&self) -> AuthMethod {
        if self.requires_mfi() {
            return AuthMethod::MfiRequired;
        }

        if self.supports_unified_pair_mfi() || self.supports_transient_pairing() {
            return AuthMethod::HomeKitTransient;
        }

        if self.requires_fairplay() {
            return AuthMethod::FairPlay;
        }

        AuthMethod::None
    }
}

impl From<u64> for Features {
    fn from(value: u64) -> Self {
        Self::from_raw(value)
    }
}
