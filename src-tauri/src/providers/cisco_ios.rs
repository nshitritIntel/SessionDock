use crate::error::AppError;

use super::network_monitor::InterfaceObservation;

pub(crate) fn parse_interface(
    requested_name: &str,
    raw: &str,
) -> Result<Vec<InterfaceObservation>, AppError> {
    let first_line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
        .ok_or_else(|| AppError::Validation("Cisco IOS interface output was empty".into()))?;
    let (reported_name, state) = first_line.split_once(" is ").ok_or_else(|| {
        AppError::Validation("Cisco IOS output did not contain interface state".into())
    })?;
    if !reported_name.eq_ignore_ascii_case(requested_name) {
        return Err(AppError::Validation(format!(
            "Cisco IOS returned interface '{reported_name}' instead of '{requested_name}'"
        )));
    }
    let (admin_status, oper_status) = parse_state(state);
    let speed_bps = number_after(raw, "BW ").and_then(|value| value.checked_mul(1_000));

    Ok(vec![InterfaceObservation {
        name: reported_name.to_string(),
        admin_status: Some(admin_status.into()),
        oper_status: Some(oper_status.into()),
        speed_bps,
        input_rate_bps: number_after(raw, "5 minute input rate "),
        output_rate_bps: number_after(raw, "5 minute output rate "),
        input_errors: number_before(raw, " input errors"),
        output_errors: number_before(raw, " output errors"),
        input_discards: input_queue_drops(raw),
        output_discards: number_after(raw, "Total output drops: "),
        neighbor: None,
        transceiver_status: None,
    }])
}

fn parse_state(state: &str) -> (&'static str, &'static str) {
    let state = state.to_ascii_lowercase();
    let admin_status = if state.starts_with("administratively down") {
        "disabled"
    } else {
        "enabled"
    };
    let oper_status = if state.contains("line protocol is up") {
        "up"
    } else if state.contains("err-disabled") {
        "errdisabled"
    } else {
        "down"
    };
    (admin_status, oper_status)
}

fn number_after(raw: &str, label: &str) -> Option<i64> {
    let suffix = raw.split_once(label)?.1;
    let digits = suffix.trim_start().chars().take_while(char::is_ascii_digit);
    digits.collect::<String>().parse().ok()
}

fn number_before(raw: &str, label: &str) -> Option<i64> {
    let prefix = raw.split_once(label)?.0;
    let digits = prefix
        .chars()
        .rev()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.chars().rev().collect::<String>().parse().ok()
}

fn input_queue_drops(raw: &str) -> Option<i64> {
    let suffix = raw.split_once("Input queue: ")?.1;
    suffix
        .split_whitespace()
        .next()?
        .split('/')
        .nth(2)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::parse_interface;

    const HEALTHY: &str = r#"GigabitEthernet1/0/1 is up, line protocol is up (connected)
  Hardware is Gigabit Ethernet, address is 0011.2233.4455
  MTU 1500 bytes, BW 1000000 Kbit/sec, DLY 10 usec,
  Input queue: 0/75/0/0 (size/max/drops/flushes); Total output drops: 0
  5 minute input rate 1200 bits/sec, 2 packets/sec
  5 minute output rate 2400 bits/sec, 3 packets/sec
     0 input errors, 0 CRC, 0 frame, 0 overrun, 0 ignored
     0 output errors, 0 collisions, 0 interface resets"#;

    #[test]
    fn parses_ios_interface_state_rates_and_zero_counters() {
        let observations = parse_interface("GigabitEthernet1/0/1", HEALTHY).unwrap();
        let interface = &observations[0];
        assert_eq!(interface.oper_status.as_deref(), Some("up"));
        assert_eq!(interface.speed_bps, Some(1_000_000_000));
        assert_eq!(interface.input_rate_bps, Some(1_200));
        assert_eq!(interface.output_rate_bps, Some(2_400));
        assert_eq!(interface.input_errors, Some(0));
        assert_eq!(interface.output_discards, Some(0));
    }

    #[test]
    fn parses_ios_down_state_and_nonzero_counters() {
        let raw = HEALTHY
            .replace(
                "is up, line protocol is up",
                "is down, line protocol is down",
            )
            .replace("0 input errors", "7 input errors")
            .replace("Total output drops: 0", "Total output drops: 9");
        let interface = parse_interface("GigabitEthernet1/0/1", &raw)
            .unwrap()
            .remove(0);
        assert_eq!(interface.oper_status.as_deref(), Some("down"));
        assert_eq!(interface.input_errors, Some(7));
        assert_eq!(interface.output_discards, Some(9));
    }

    #[test]
    fn rejects_empty_malformed_and_wrong_interface_output() {
        assert!(parse_interface("GigabitEthernet1/0/1", "").is_err());
        assert!(parse_interface("GigabitEthernet1/0/1", "invalid").is_err());
        assert!(parse_interface(
            "GigabitEthernet1/0/1",
            HEALTHY.replace("1/0/1", "1/0/2").as_str()
        )
        .is_err());
    }
}
