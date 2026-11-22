//! Network, mapped-drive and WSL discovery for Ferrix Explorer Tranche 4.

use std::{path::PathBuf, process::Command};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkLocation {
    pub label: String,
    pub path: PathBuf,
}

pub fn parse_wsl_output(output: &str) -> Vec<NetworkLocation> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| NetworkLocation {
            label: line.trim_start_matches('*').trim().to_owned(),
            path: PathBuf::from(format!(
                r"\\wsl.localhost\{}",
                line.trim_start_matches('*').trim()
            )),
        })
        .collect()
}

pub fn parse_net_use_output(output: &str) -> Vec<NetworkLocation> {
    output
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() >= 3 && fields[1].chars().count() == 2 && fields[1].ends_with(':') {
                Some(NetworkLocation {
                    label: format!("Network ({})", fields[1]),
                    path: PathBuf::from(fields[2].replace('/', "\\")),
                })
            } else {
                None
            }
        })
        .collect()
}

pub fn discover_mapped_drives() -> Vec<NetworkLocation> {
    let Ok(output) = Command::new("net.exe").args(["use"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_net_use_output(&String::from_utf8_lossy(&output.stdout))
}

pub fn discover_wsl() -> Vec<NetworkLocation> {
    let Ok(output) = Command::new("wsl.exe").args(["--list", "--quiet"]).output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_wsl_output(&String::from_utf16_lossy(&decode_utf16(
        output.stdout.as_slice(),
    )))
}

fn decode_utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wsl_distros_from_quiet_output() {
        let locations = parse_wsl_output("Ubuntu\r\nDebian\r\n");
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[0].label, "Ubuntu");
        assert_eq!(locations[0].path, PathBuf::from(r"\\wsl.localhost\Ubuntu"));
    }

    #[test]
    fn ignores_non_mapped_net_use_lines() {
        let output = "Status       Local     Remote                    Network\r\n-------------------------------------------------------------------------------\r\nOK           Z:        \\\\server\\share            Microsoft Windows Network\r\n";
        let locations = parse_net_use_output(output);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].path, PathBuf::from(r"\\server\share"));
    }
}
