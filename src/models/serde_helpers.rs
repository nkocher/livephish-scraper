use serde::{Deserialize, Deserializer};

/// Deserialize a value that may be an int, string, or null into i64.
/// Matches Python's `_safe_int()` behavior.
pub fn deserialize_safe_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value_to_i64(&value, 0))
}

/// Deserialize with a custom default of 1 (used for disc_num).
pub fn deserialize_safe_i64_default_1<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value_to_i64(&value, 1))
}

/// Deserialize a value that may be a string, int, or null into String.
/// Matches Python's `_safe_str()` behavior.
pub fn deserialize_safe_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value_to_string(&value))
}

fn value_to_i64(value: &serde_json::Value, default: i64) -> i64 {
    match value {
        serde_json::Value::Number(n) => n.as_i64().unwrap_or(default),
        serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(default),
        serde_json::Value::Null => default,
        _ => default,
    }
}

fn value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct TestInt {
        #[serde(deserialize_with = "deserialize_safe_i64")]
        value: i64,
    }

    #[derive(Deserialize)]
    struct TestString {
        #[serde(deserialize_with = "deserialize_safe_string")]
        value: String,
    }

    #[test]
    fn test_safe_i64_from_int() {
        let t: TestInt = serde_json::from_str(r#"{"value": 42}"#).unwrap();
        assert_eq!(t.value, 42);
    }

    #[test]
    fn test_safe_i64_from_string() {
        let t: TestInt = serde_json::from_str(r#"{"value": "42"}"#).unwrap();
        assert_eq!(t.value, 42);
    }

    #[test]
    fn test_safe_i64_from_null() {
        let t: TestInt = serde_json::from_str(r#"{"value": null}"#).unwrap();
        assert_eq!(t.value, 0);
    }

    #[test]
    fn test_safe_i64_from_garbage_string() {
        let t: TestInt = serde_json::from_str(r#"{"value": "abc"}"#).unwrap();
        assert_eq!(t.value, 0);
    }

    #[test]
    fn test_safe_string_from_string() {
        let t: TestString = serde_json::from_str(r#"{"value": "hello"}"#).unwrap();
        assert_eq!(t.value, "hello");
    }

    #[test]
    fn test_safe_string_from_null() {
        let t: TestString = serde_json::from_str(r#"{"value": null}"#).unwrap();
        assert_eq!(t.value, "");
    }

    #[test]
    fn test_safe_string_from_number() {
        let t: TestString = serde_json::from_str(r#"{"value": 123}"#).unwrap();
        assert_eq!(t.value, "123");
    }
}
