//! Oryn core — document model, markdown serialization, shared across GUI and mobile frontends.

pub fn hello() -> &'static str {
    "Hello from oryn-rs"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_returns_expected_string() {
        assert_eq!(hello(), "Hello from oryn-rs");
    }
}
