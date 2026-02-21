use crate::api::NugsApi;

use super::Service;

/// Owns both API clients and dispatches based on Service tag.
///
/// Created once at startup. When LivePhish credentials are available,
/// `livephish` is `Some(...)`. Otherwise Phish shows fall back to nugs.net.
pub struct ServiceRouter {
    pub nugs: NugsApi,
    pub livephish: Option<NugsApi>,
}

impl ServiceRouter {
    /// Get the API client for the given service.
    ///
    /// Falls back to nugs if LivePhish is requested but unavailable.
    pub fn api_for(&mut self, service: Service) -> &mut NugsApi {
        if service == Service::LivePhish {
            if let Some(ref mut lp) = self.livephish {
                return lp;
            }
        }
        &mut self.nugs
    }

    /// Whether a LivePhish API client is available.
    pub fn has_livephish(&self) -> bool {
        self.livephish.is_some()
    }
}
