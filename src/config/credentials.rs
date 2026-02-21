use super::KEYRING_SERVICE;
use crate::config::Config;
use crate::service::Service;

/// Get password from system keyring using a specific keyring service name.
/// Returns None if keyring is unavailable or password not stored.
pub fn get_keyring_password_for(email: &str, keyring_service: &str) -> Option<String> {
    let entry = keyring::Entry::new(keyring_service, email).ok()?;
    entry.get_password().ok()
}

/// Get password from system keyring (nugs service — backward compat).
/// Returns None if keyring is unavailable or password not stored.
#[allow(dead_code)] // Used by nugs service; kept for backward compat
pub fn get_keyring_password(email: &str) -> Option<String> {
    get_keyring_password_for(email, KEYRING_SERVICE)
}

/// Save password to system keyring using a specific keyring service name.
/// Returns true on success, false if keyring unavailable.
pub fn set_keyring_password_for(email: &str, password: &str, keyring_service: &str) -> bool {
    match keyring::Entry::new(keyring_service, email) {
        Ok(entry) => entry.set_password(password).is_ok(),
        Err(_) => false,
    }
}

/// Save password to system keyring (nugs service — backward compat).
/// Returns true on success, false if keyring unavailable.
pub fn set_keyring_password(email: &str, password: &str) -> bool {
    set_keyring_password_for(email, password, KEYRING_SERVICE)
}

/// Get credentials for a specific service using config and keyring.
/// Returns (email, password) or an error message.
#[allow(dead_code)] // Phase 5: used by multi-service router
pub fn get_credentials_for_service(
    config: &Config,
    service: Service,
) -> Result<(String, String), String> {
    let email = config.email_for(service);
    let keyring_service = service.config().keyring_service;
    get_credentials_with_keyring(email, keyring_service)
}

/// Internal helper: resolve credentials given an email and keyring service name.
fn get_credentials_with_keyring(
    email: &str,
    keyring_service: &str,
) -> Result<(String, String), String> {
    if email.is_empty() {
        return Err("Email not configured. Run 'nugs config' to set up.".to_string());
    }

    // Try keyring first
    if let Some(password) = get_keyring_password_for(email, keyring_service) {
        return Ok((email.to_string(), password));
    }

    // Fall back to terminal prompt
    let password = rpassword::prompt_password(format!("Password for {email}: "))
        .map_err(|e| format!("Failed to read password: {e}"))?;

    if password.is_empty() {
        return Err("Password required.".to_string());
    }

    Ok((email.to_string(), password))
}

/// Get credentials: email from config, password from keyring or rpassword prompt.
/// Returns (email, password) or error message.
/// Uses the nugs keyring service — kept for backward compatibility.
pub fn get_credentials(email: &str) -> Result<(String, String), String> {
    get_credentials_with_keyring(email, KEYRING_SERVICE)
}
