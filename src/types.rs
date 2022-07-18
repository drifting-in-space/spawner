use serde::{Deserialize, Serialize};
use std::fmt::Display;

const RESOURCE_PREFIX: &str = "spawner-";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DroneId(u32);

impl DroneId {
    #[must_use]
    pub fn new(id: u32) -> Self {
        DroneId(id)
    }

    #[must_use]
    pub fn id(&self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn id_i32(&self) -> i32 {
        self.0 as i32
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct BackendId(String);

impl Display for BackendId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl BackendId {
    #[must_use]
    pub fn new(id: String) -> Self {
        BackendId(id)
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn to_resource_name(&self) -> String {
        format!("{}{}", RESOURCE_PREFIX, self.0)
    }

    #[must_use]
    pub fn from_resource_name(resource_name: &str) -> Option<Self> {
        resource_name
            .strip_prefix(RESOURCE_PREFIX)
            .map(|d| BackendId(d.to_string()))
    }
}
