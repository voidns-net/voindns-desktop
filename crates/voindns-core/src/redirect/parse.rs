//! Pure parsers for OS command/file output. Split out of the `cfg`-gated
//! platform modules so every platform's parsing logic is unit-testable on any
//! host (the Linux CI runner exercises all three).

// Each parser is only *called* on its own OS, so the others look dead elsewhere.
#![allow(dead_code)]

use std::net::IpAddr;

/// Linux: the interface owning the default route, from `/proc/net/route`
/// (destination column `00000000`).
pub fn parse_default_route_iface(table: &str) -> Option<String> {
    for line in table.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let iface = cols.next()?;
        let dest = cols.next()?;
        if dest == "00000000" {
            return Some(iface.to_string());
        }
    }
    None
}

/// Windows: connected interface aliases from `netsh interface show interface`.
/// Columns: `Admin State | State | Type | Interface Name…`.
pub fn parse_netsh_interfaces(stdout: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in stdout.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 4 && cols[1].eq_ignore_ascii_case("connected") {
            names.push(cols[3..].join(" "));
        }
    }
    names
}

/// macOS: service names from `networksetup -listallnetworkservices`. The first
/// line is an informational header; a leading `*` marks a disabled service.
pub fn parse_network_services(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .skip(1)
        .map(|l| l.trim_start_matches('*').trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// macOS: DNS server IPs from `networksetup -getdnsservers <svc>`. Returns empty
/// when none are set (the command prints a sentence instead of IPs).
pub fn parse_dns_servers(stdout: &str) -> Vec<String> {
    if stdout.contains("aren't any DNS Servers") {
        return Vec::new();
    }
    stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| l.parse::<IpAddr>().is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Linux ---

    #[test]
    fn linux_default_route_picks_zero_destination() {
        let table = concat!(
            "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n",
            "wlan0\t00000000\t0102A8C0\t0003\t0\t0\t600\t00000000\n",
            "wlan0\t0002A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\n",
            "docker0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF",
        );
        assert_eq!(parse_default_route_iface(table).as_deref(), Some("wlan0"));
    }

    #[test]
    fn linux_default_route_none_when_absent() {
        let table = "Iface\tDestination\tGateway\nlo\t0000007F\t00000000";
        assert_eq!(parse_default_route_iface(table), None);
    }

    // --- Windows ---

    #[test]
    fn windows_parses_connected_interfaces() {
        let out = concat!(
            "\n\n",
            "Admin State    State          Type             Interface Name\n",
            "-------------------------------------------------------------------------\n",
            "Enabled        Connected      Dedicated        Ethernet\n",
            "Enabled        Connected      Dedicated        Wi-Fi\n",
            "Enabled        Disconnected   Dedicated        Ethernet 2",
        );
        let got = parse_netsh_interfaces(out);
        assert_eq!(got, vec!["Ethernet".to_string(), "Wi-Fi".to_string()]);
    }

    #[test]
    fn windows_interface_name_with_spaces() {
        let out = "Enabled        Connected      Dedicated        Local Area Connection 3";
        assert_eq!(parse_netsh_interfaces(out), vec!["Local Area Connection 3"]);
    }

    #[test]
    fn windows_empty_when_none_connected() {
        let out = "Enabled        Disconnected   Dedicated        Ethernet";
        assert!(parse_netsh_interfaces(out).is_empty());
    }

    // --- macOS ---

    #[test]
    fn macos_parses_services_skipping_header_and_star() {
        let out = concat!(
            "An asterisk (*) denotes that a network service is disabled.\n",
            "Wi-Fi\n",
            "*Bluetooth PAN\n",
            "Thunderbolt Bridge",
        );
        let got = parse_network_services(out);
        assert_eq!(
            got,
            vec![
                "Wi-Fi".to_string(),
                "Bluetooth PAN".to_string(),
                "Thunderbolt Bridge".to_string()
            ]
        );
    }

    #[test]
    fn macos_dns_servers_listed() {
        let out = "1.1.1.1\n8.8.8.8";
        assert_eq!(parse_dns_servers(out), vec!["1.1.1.1", "8.8.8.8"]);
    }

    #[test]
    fn macos_dns_servers_none() {
        let out = "There aren't any DNS Servers set on Wi-Fi.";
        assert!(parse_dns_servers(out).is_empty());
    }
}
