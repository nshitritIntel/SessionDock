use crate::error::AppError;

use super::{arista_eos, cisco_ios};

const MAX_INTERFACE_NAME_LENGTH: usize = 64;

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
pub(crate) struct MonitorCommands {
    pub(crate) interface: String,
    pub(crate) logs: String,
}

struct NetworkMonitorProvider {
    id: &'static str,
    interface_command: fn(&str) -> String,
    log_command: fn(&str) -> String,
    parser: fn(&str, &str) -> Result<Vec<InterfaceObservation>, AppError>,
}

const PROVIDERS: &[NetworkMonitorProvider] = &[
    NetworkMonitorProvider {
        id: "arista-eos",
        interface_command: |name| format!("show interfaces {name} | json"),
        log_command: |name| format!("show logging | grep {name}"),
        parser: |_, raw| arista_eos::parse_interfaces(raw),
    },
    NetworkMonitorProvider {
        id: "cisco-ios",
        interface_command: |name| format!("show interfaces {name}"),
        log_command: |name| format!("show logging | include {name}"),
        parser: cisco_ios::parse_interface,
    },
];

pub(crate) fn is_supported(provider_id: &str) -> bool {
    provider(provider_id).is_some()
}

pub(crate) fn validate_interface_name(value: &str) -> Result<&str, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_INTERFACE_NAME_LENGTH
        || value.starts_with('-')
        || value.contains("..")
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/' | b'.' | b':')
        })
    {
        return Err(AppError::Validation(
            "Enter one valid interface name, for example Ethernet15 or GigabitEthernet1/0/1".into(),
        ));
    }
    Ok(value)
}

pub(crate) fn commands(
    provider_id: &str,
    interface_name: &str,
) -> Result<MonitorCommands, AppError> {
    let provider = provider(provider_id).ok_or_else(unsupported_provider)?;
    let interface_name = validate_interface_name(interface_name)?;
    Ok(MonitorCommands {
        interface: (provider.interface_command)(interface_name),
        logs: (provider.log_command)(interface_name),
    })
}

pub(crate) fn parse_interfaces(
    provider_id: &str,
    interface_name: &str,
    raw: &str,
) -> Result<Vec<InterfaceObservation>, AppError> {
    let provider = provider(provider_id).ok_or_else(unsupported_provider)?;
    (provider.parser)(interface_name, raw)
}

fn provider(provider_id: &str) -> Option<&'static NetworkMonitorProvider> {
    PROVIDERS.iter().find(|provider| provider.id == provider_id)
}

fn unsupported_provider() -> AppError {
    AppError::Validation(
        "Unsupported Network Health provider; select Arista EOS or Cisco IOS/IOS-XE".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::{commands, is_supported};

    #[test]
    fn builds_only_allowlisted_provider_commands() {
        assert_eq!(
            commands("arista-eos", "Ethernet15").unwrap().logs,
            "show logging | grep Ethernet15"
        );
        assert_eq!(
            commands("cisco-ios", "GigabitEthernet1/0/1").unwrap(),
            super::MonitorCommands {
                interface: "show interfaces GigabitEthernet1/0/1".into(),
                logs: "show logging | include GigabitEthernet1/0/1".into(),
            }
        );
        assert!(commands("cisco-ios", "Ethernet1; reload").is_err());
        assert!(commands("juniper-junos", "ge-0/0/1").is_err());
        assert!(is_supported("arista-eos"));
        assert!(is_supported("cisco-ios"));
        assert!(!is_supported("generic-ssh"));
    }
}
