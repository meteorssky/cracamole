use std::collections::HashMap;

/// Connection parameters from the `connect` instruction, keyed by argument name.
/// Empty values are treated as "not set", as in the C implementation.
#[derive(Debug, Clone, Default)]
pub struct Params(HashMap<String, String>);

impl Params {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self(
            pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }

    pub fn insert(&mut self, k: impl Into<String>, v: impl Into<String>) {
        self.0.insert(k.into(), v.into());
    }

    /// Value if present and non-empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    /// `true` only for the literal string "true" (Guacamole convention).
    pub fn get_bool(&self, name: &str) -> bool {
        self.get(name) == Some("true")
    }

    pub fn get_u32(&self, name: &str) -> Option<u32> {
        self.get(name).and_then(|s| s.trim().parse().ok())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
