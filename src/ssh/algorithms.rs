use russh::Preferred;

/// Build a `russh::Preferred` with our desired algorithm ordering.
/// We just use the russh defaults which include all supported algorithms.
pub fn preferred_algorithms() -> Preferred {
    Preferred::default()
}
