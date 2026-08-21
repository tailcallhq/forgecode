/// A floating-point newtype that serializes to two decimal places, preventing
/// `toml_edit` from emitting noisy bit-pattern approximations such as
/// `0.10000000149011612` or `0.20000000000000001`.
///
/// The inner value is stored as `f64`. When used for fields that ultimately
/// require `f32`, callers should cast via `value() as f32`.
pub struct Decimal(pub f64);

impl Decimal {
    /// Returns the inner `f64` value.
    pub fn value(&self) -> f64 {
        self.0
    }
}

impl std::fmt::Debug for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Clone for Decimal {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for Decimal {}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl Default for Decimal {
    fn default() -> Self {
        Self(0.0)
    }
}

impl From<f64> for Decimal {
    fn from(v: f64) -> Self {
        Self(v)
    }
}

impl From<f32> for Decimal {
    fn from(v: f32) -> Self {
        Self(v as f64)
    }
}

impl From<Decimal> for f64 {
    fn from(d: Decimal) -> Self {
        d.0
    }
}

impl schemars::JsonSchema for Decimal {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        f64::schema_name()
    }

    fn json_schema(r#gen: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        f64::json_schema(r#gen)
    }
}

impl fake::Dummy<fake::Faker> for Decimal {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(_: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Self((0.0f64..2.0f64).fake_with_rng(rng))
    }
}

impl serde::Serialize for Decimal {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let formatted: f64 = format!("{:.2}", self.0).parse().unwrap();
        serializer.serialize_f64(formatted)
    }
}

impl<'de> serde::Deserialize<'de> for Decimal {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(f64::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Reusable fixture producing a `Decimal` from any `f64`.
    fn fixture(value: f64) -> Decimal {
        Decimal::from(value)
    }

    #[test]
    fn test_value_returns_inner() {
        let fixture = fixture(1.25);

        let actual = fixture.value();

        let expected = 1.25;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_default_is_zero() {
        let actual = Decimal::default();

        let expected = fixture(0.0);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_from_f32_widens_to_f64() {
        let actual = Decimal::from(0.5f32);

        let expected = fixture(0.5);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_into_f64_roundtrip() {
        let fixture = fixture(3.75);

        let actual: f64 = fixture.into();

        let expected = 3.75;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_debug_delegates_to_inner_f64() {
        let fixture = fixture(0.1);

        let actual = format!("{fixture:?}");

        let expected = format!("{:?}", 0.1f64);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_partial_ord_compares_by_inner_value() {
        let setup = (fixture(0.1), fixture(0.2));

        let actual = setup.0.partial_cmp(&setup.1);

        let expected = Some(std::cmp::Ordering::Less);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_serialize_rounds_to_two_decimals() {
        let fixture = fixture(0.100_000_001_490_116_12);

        let actual = serde_json::to_string(&fixture).unwrap();

        let expected = "0.1".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_serialize_rounds_half_away_from_zero() {
        let fixture = fixture(1.567);

        let actual = serde_json::to_string(&fixture).unwrap();

        let expected = "1.57".to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_deserialize_preserves_full_precision() {
        let setup = "0.12345";

        let actual: Decimal = serde_json::from_str(setup).unwrap();

        let expected = fixture(0.12345);
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_serialize_deserialize_roundtrip_is_rounded() {
        let fixture = fixture(2.349);

        let actual: Decimal =
            serde_json::from_str(&serde_json::to_string(&fixture).unwrap()).unwrap();

        let expected = Decimal::from(2.35);
        assert_eq!(actual, expected);
    }
}
