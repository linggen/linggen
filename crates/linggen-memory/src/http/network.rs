//! Saved network choice in the memory configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    #[default]
    Lan,
    Local,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lan_is_the_default() {
        assert_eq!(Network::default(), Network::Lan);
    }

    #[test]
    fn the_setting_round_trips_as_a_word() {
        let v = serde_json::to_value(Network::Local).unwrap();
        assert_eq!(v, serde_json::json!("local"));
        let n: Network = serde_json::from_value(serde_json::json!("lan")).unwrap();
        assert_eq!(n, Network::Lan);
    }
}
