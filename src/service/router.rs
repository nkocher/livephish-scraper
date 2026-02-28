use crate::api::NugsApi;

use super::Service;

/// Owns all API clients and dispatches based on Service tag.
///
/// Created once at startup. When LivePhish credentials are available,
/// `livephish` is `Some(...)`. Otherwise Phish shows fall back to nugs.net.
/// When a Google API key is available, `bman` is `Some(...)` for Grateful
/// Dead / JGB shows from Bman's Google Drive archive.
pub struct ServiceRouter {
    pub nugs: NugsApi,
    pub livephish: Option<NugsApi>,
    pub bman: Option<crate::bman::BmanApi>,
}

impl ServiceRouter {
    /// Get the NugsApi client for the given service.
    ///
    /// Falls back to nugs if LivePhish is requested but unavailable.
    /// Panics if called with `Service::Bman` — use `bman_api()` instead.
    pub fn api_for(&mut self, service: Service) -> &mut NugsApi {
        match service {
            Service::Bman => panic!(
                "BmanApi is not a NugsApi — use router.bman_api() for Bman shows"
            ),
            Service::LivePhish => {
                if let Some(ref mut lp) = self.livephish {
                    return lp;
                }
                &mut self.nugs
            }
            Service::Nugs => &mut self.nugs,
        }
    }

    /// Whether a LivePhish API client is available.
    pub fn has_livephish(&self) -> bool {
        self.livephish.is_some()
    }

    /// Whether a Bman (Google Drive) API client is available.
    #[allow(dead_code)] // Phase 4: used by catalog integration
    pub fn has_bman(&self) -> bool {
        self.bman.is_some()
    }

    /// Get a mutable reference to the BmanApi, if available.
    pub fn bman_api(&mut self) -> Option<&mut crate::bman::BmanApi> {
        self.bman.as_mut()
    }
}
