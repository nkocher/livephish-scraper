pub mod auth;
pub mod catalog_api;
pub mod client;
pub mod error;
pub mod stream_api;

// API constants — kept as convenience aliases for the default (Nugs) service.
// New code should use service.config() directly.
pub const RATE_LIMIT_DELAY_MS: u64 = 500;
pub const MAX_RETRIES: usize = 3;

// Re-exports
pub use client::NugsApi;

#[cfg(test)]
mod tests;
