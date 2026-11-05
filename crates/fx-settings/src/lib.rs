//! Small persistent settings used by the native Tranche 0 host.
//!
//! Settings live under `%APPDATA%\FerrixExplorer\` in the standard (non-portable) mode described by
//! PRD 10.10. The format is deliberately trivial (`key=value` lines) so a partially written file
//! can be rejected without a parser dependency.

use std::{fs, io, path::PathBuf};

pub mod view_state;

const DEFAULT_WIDTH: i32 = 1100;
const DEFAULT_HEIGHT: i32 = 720;
const MAX_WINDOW_DIMENSION: i32 = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowSize {
    pub width: i32,
    pub height: i32,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

impl WindowSize {
    fn validate(self) -> io::Result<Self> {
        if (1..=MAX_WINDOW_DIMENSION).contains(&self.width)
            && (1..=MAX_WINDOW_DIMENSION).contains(&self.height)
        {
            Ok(self)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "saved window dimensions are outside the supported range",
            ))
        }
    }
}

/// `%APPDATA%\FerrixExplorer\window.ini`, matching the standard data location in PRD 10.10.
fn settings_path() -> io::Result<PathBuf> {
    let base_dirs = directories::BaseDirs::new()
        .ok_or_else(|| io::Error::other("AppData could not be resolved"))?;
    Ok(base_dirs
        .config_dir()
        .join("FerrixExplorer")
        .join("window.ini"))
}

/// Load the saved outer window dimensions, defaulting only when no settings file exists.
pub fn load_window_size() -> io::Result<WindowSize> {
    let contents = match fs::read_to_string(settings_path()?) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(WindowSize::default()),
        Err(error) => return Err(error),
    };
    parse_window_size(&contents)
}

fn parse_window_size(contents: &str) -> io::Result<WindowSize> {
    let mut width = None;
    let mut height = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "width" && key != "height" {
            continue;
        }
        let parsed = value.trim().parse::<i32>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid {key} value: {error}"),
            )
        })?;
        if key == "width" {
            width = Some(parsed);
        } else {
            height = Some(parsed);
        }
    }

    WindowSize {
        width: width.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing width"))?,
        height: height
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing height"))?,
    }
    .validate()
}

/// Persist the outer window dimensions for the next launch.
pub fn save_window_size(size: WindowSize) -> io::Result<()> {
    let size = size.validate()?;
    let path = settings_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    fs::write(
        path,
        format!("width={}\nheight={}\n", size.width, size.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_reasonable_desktop_dimensions() {
        assert_eq!(
            WindowSize::default(),
            WindowSize {
                width: 1100,
                height: 720,
            }
        );
    }

    #[test]
    fn parses_dimensions_in_any_order_and_ignores_unknown_keys() {
        let parsed = parse_window_size("height=900\nfuture-option=yes\nwidth=1600\n").unwrap();
        assert_eq!(
            parsed,
            WindowSize {
                width: 1600,
                height: 900
            }
        );
    }

    #[test]
    fn rejects_missing_or_malformed_dimensions() {
        assert_eq!(
            parse_window_size("width=800\n").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            parse_window_size("width=wide\nheight=600\n")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn rejects_non_positive_or_unreasonably_large_dimensions() {
        assert!(
            (WindowSize {
                width: 0,
                height: 720
            })
            .validate()
            .is_err()
        );
        assert!(
            (WindowSize {
                width: 1100,
                height: -1
            })
            .validate()
            .is_err()
        );
        assert!(
            (WindowSize {
                width: 1100,
                height: MAX_WINDOW_DIMENSION + 1,
            })
            .validate()
            .is_err()
        );
        assert!(parse_window_size("width=0\nheight=720\n").is_err());
    }
}
