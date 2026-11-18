use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::AppError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InterfaceObservation {
    pub(crate) name: String,
    pub(crate) admin_status: Option<String>,
    pub(crate) oper_status: Option<String>,
    pub(crate) speed_bps: Option<i64>,
    pub(crate) input_rate_bps: Option<i64>,
    pub(crate) output_rate_bps: Option<i64>,
    pub(crate) input_errors: Option<i64>,
    pub(crate) output_errors: Option<i64>,
    pub(crate) input_discards: Option<i64>,
    pub(crate) output_discards: Option<i64>,
    pub(crate) neighbor: Option<String>,
    pub(crate) transceiver_status: Option<String>,
}

pub(crate) fn parse_interfaces(raw: &str) -> Result<Vec<InterfaceObservation>, AppError> {
    let root: Value = serde_json::from_str(raw).map_err(|error| {
        AppError::Validation(format!("Invalid Arista EOS JSON output: {error}"))
    })?;
    let interfaces = object_at(&root, &["interfaces", "interfaceStatuses"]).ok_or_else(|| {
        AppError::Validation("Arista EOS output did not contain interfaces".into())
    })?;
    let mut observations = interfaces
        .iter()
        .map(|(name, value)| parse_interface(name, value))
        .collect::<Vec<_>>();
    observations.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(observations)
}

fn parse_interface(name: &str, value: &Value) -> InterfaceObservation {
    let counters = value.get("interfaceCounters").unwrap_or(value);
    InterfaceObservation {
        name: name.to_string(),
        admin_status: string_at(value, &["adminStatus", "interfaceStatus"]),
        oper_status: string_at(value, &["lineProtocolStatus", "linkStatus", "operStatus"]),
        speed_bps: integer_at(value, &["bandwidth", "speed"]),
        input_rate_bps: integer_at(value, &["inBitsRate", "inputRate"]),
        output_rate_bps: integer_at(value, &["outBitsRate", "outputRate"]),
        input_errors: integer_at(counters, &["inErrors", "inputErrors"]),
        output_errors: integer_at(counters, &["outErrors", "outputErrors"]),
        input_discards: integer_at(counters, &["inDiscards", "inputDiscards"]),
        output_discards: integer_at(counters, &["outDiscards", "outputDiscards"]),
        neighbor: string_at(value, &["neighbor", "lldpNeighbor"]),
        transceiver_status: string_at(value, &["transceiverStatus"]),
    }
}

fn object_at<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    keys.iter().find_map(|key| value.get(*key)?.as_object())
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_str())
        .map(str::to_string)
}

fn integer_at(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let value = value.get(*key)?;
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
    })
}

#[cfg(test)]
mod tests {
    use super::parse_interfaces;

    #[test]
    fn parses_eos_interface_state_and_counters() {
        let observations = parse_interfaces(r#"{
          "interfaces": {
            "Ethernet1": {
              "interfaceStatus": "connected",
              "lineProtocolStatus": "up",
              "bandwidth": 100000000000,
              "inBitsRate": 1200,
              "outBitsRate": 2400,
              "interfaceCounters": { "inErrors": 2, "outErrors": 3, "inDiscards": 4, "outDiscards": 5 }
            }
          }
        }"#).unwrap();

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].name, "Ethernet1");
        assert_eq!(observations[0].oper_status.as_deref(), Some("up"));
        assert_eq!(observations[0].speed_bps, Some(100_000_000_000));
        assert_eq!(observations[0].input_errors, Some(2));
        assert_eq!(observations[0].output_discards, Some(5));
    }

    #[test]
    fn rejects_non_json_and_unknown_shapes() {
        assert!(parse_interfaces("not-json").is_err());
        assert!(parse_interfaces("{}").is_err());
    }
}
