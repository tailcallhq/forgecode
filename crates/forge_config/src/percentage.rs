use crate::Decimal;

/// A percentage value constrained to `[0.0, 1.0]`, built on top of [`Decimal`]
/// to inherit its two-decimal-place serialization and clean TOML output.
///
/// Validation is enforced at deserialization time, so any config file with an
/// out-of-range value is rejected with a descriptive error.
#[derive(Default)]
pub struct Percentage(Decimal);

impl Percentage {
    const MIN: f64 = 0.0;
    const MAX: f64 = 1.0;

    /// Construct a validated `Percentage`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `value` is outside `[0.0, 1.0]`.
    pub fn new(value: f64) -> Result<Self, String> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(Decimal(value)))
        } else {
            Err(format!(
                "value must be between {} and {}, got {value}",
                Self::MIN,
                Self::MAX
            ))
        }
    }

    /// Returns the inner `f64` value.
    pub fn value(&self) -> f64 {
        self.0.value()
    }
}

impl std::fmt::Debug for Percentage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Clone for Percentage {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for Percentage {}

impl PartialEq for Percentage {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl PartialOrd for Percentage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl From<f64> for Percentage {
    fn from(v: f64) -> Self {
        Self(Decimal::from(v))
    }
}

impl From<Percentage> for f64 {
    fn from(p: Percentage) -> Self {
        p.0.into()
    }
}

impl schemars::JsonSchema for Percentage {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        Decimal::schema_name()
    }

    fn json_schema(r#gen: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        Decimal::json_schema(r#gen)
    }
}

impl fake::Dummy<fake::Faker> for Percentage {
    fn dummy_with_rng<R: fake::RngExt + ?Sized>(_: &fake::Faker, rng: &mut R) -> Self {
        use fake::Fake;
        Self(Decimal((0.0f64..=1.0f64).fake_with_rng::<f64, R>(rng)))
    }
}

impl serde::Serialize for Percentage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Percentage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let decimal = Decimal::deserialize(deserializer)?;
        Self::new(decimal.value()).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_percentage_valid_range() {
        assert!(Percentage::new(0.0).is_ok());
        assert!(Percentage::new(0.5).is_ok());
        assert!(Percentage::new(1.0).is_ok());
    }

    #[test]
    fn test_percentage_rejects_out_of_range() {
        assert!(Percentage::new(-0.1).is_err());
        assert!(Percentage::new(1.1).is_err());
    }

    #[test]
    fn test_percentage_serializes_to_2dp() {
        #[derive(serde::Serialize)]
        struct Fixture {
            value: Percentage,
        }
        let fixture = Fixture { value: Percentage::new(0.2).unwrap() };
        let actual = toml_edit::ser::to_string_pretty(&fixture).unwrap();
        let expected = "value = 0.2\n";
        assert_eq!(actual, expected);
    }
}
