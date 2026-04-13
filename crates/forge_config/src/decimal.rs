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
