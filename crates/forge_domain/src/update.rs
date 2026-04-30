use std::time::Duration;

use derive_setters::Setters;
use merge::Merge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFrequency {
    Daily,
    Weekly,
    Never,
    #[default]
    Always,
}

impl From<UpdateFrequency> for Duration {
    fn from(val: UpdateFrequency) -> Self {
        match val {
            UpdateFrequency::Daily => Duration::from_secs(60 * 60 * 24), // 1 day
            UpdateFrequency::Weekly => Duration::from_secs(60 * 60 * 24 * 7), // 1 week
            UpdateFrequency::Never => Duration::MAX,
            UpdateFrequency::Always => Duration::ZERO, // one time,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Merge, Default, JsonSchema, Setters, PartialEq)]
#[merge(strategy = merge::option::overwrite_none)]
pub struct Update {
    pub frequency: Option<UpdateFrequency>,
    pub auto_update: Option<bool>,
}
