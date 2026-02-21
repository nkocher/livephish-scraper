use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Service {
    #[default]
    Nugs,
    LivePhish,
}

/// Service-specific auth config — eliminates impossible Option states.
#[allow(dead_code)]
pub enum ServiceAuth {
    Nugs {
        sub_info_url: &'static str,
        user_info_url: &'static str,
    },
    LivePhish {
        sig_key: &'static str,
    },
}

/// Shared config fields (all services have these).
#[allow(dead_code)]
pub struct ServiceConfig {
    pub auth_url: &'static str,
    pub api_base: &'static str,
    pub client_id: &'static str,
    pub developer_key: &'static str,
    pub user_agent: &'static str,
    pub stream_user_agent: &'static str,
    pub oauth_scope: &'static str,
    pub keyring_service: &'static str,
    pub player_url: &'static str,
    pub auth: ServiceAuth,
}

impl Service {
    pub fn config(self) -> &'static ServiceConfig {
        match self {
            Service::Nugs => &NUGS_CONFIG,
            Service::LivePhish => &LIVEPHISH_CONFIG,
        }
    }
}

static NUGS_CONFIG: ServiceConfig = ServiceConfig {
    auth_url: "https://id.nugs.net/connect/token",
    api_base: "https://streamapi.nugs.net/",
    client_id: "Eg7HuH873H65r5rt325UytR5429",
    developer_key: "x7f54tgbdyc64y656thy47er4",
    user_agent: "NugsNet/3.26.724 (Android; 7.1.2; Asus; ASUS_Z01QD; Scale/2.0; en)",
    stream_user_agent: "nugsnetAndroid",
    oauth_scope: "openid profile email nugsnet:api nugsnet:legacyapi offline_access",
    keyring_service: "nugs",
    player_url: "https://play.nugs.net/",
    auth: ServiceAuth::Nugs {
        sub_info_url: "https://subscriptions.nugs.net/api/v1/me/subscriptions",
        user_info_url: "https://id.nugs.net/connect/userinfo",
    },
};

static LIVEPHISH_CONFIG: ServiceConfig = ServiceConfig {
    auth_url: "https://id.livephish.com/connect/token",
    api_base: "https://streamapi.livephish.com/",
    client_id: "Fujeij8d764ydxcnh4676scsr7f4",
    developer_key: "njeurd876frhdjxy6sxxe721",
    user_agent: "LivePhish/3.4.5.357 (Android; 7.1.2; Asus; ASUS_Z01QD)",
    stream_user_agent: "LivePhishAndroid",
    oauth_scope: "offline_access nugsnet:api nugsnet:legacyapi",
    keyring_service: "livephish",
    player_url: "https://plus.livephish.com/",
    auth: ServiceAuth::LivePhish {
        sig_key: "jdfirj8475jf_",
    },
};

pub mod router;
