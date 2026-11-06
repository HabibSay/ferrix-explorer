//! Navigation location types for Ferrix Explorer.
//!
//! A `Location` describes where the file-panel is pointed at. Variants cover local paths,
//! UNC shares, WSL distros, in-archive views, cloud providers, virtual folders, and
//! in-panel search results.

use std::fmt;
use std::path::PathBuf;

// ── LocationKind ─────────────────────────────────────────────────────────────

/// Discriminant for `Location`; used in error types and telemetry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationKind {
    Local,
    Unc,
    Wsl,
    Search,
    Archive,
    Cloud,
    Virtual,
}

// ── Location ──────────────────────────────────────────────────────────────────

/// The navigation target of a file panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Location {
    /// A local drive path, e.g. `C:\Users\alice`.
    Local(PathBuf),
    /// A UNC share, e.g. `\\server\share\subdir`.
    Unc {
        server: String,
        share: String,
        path: PathBuf,
    },
    /// A WSL distro path, e.g. `\\wsl$\Ubuntu\home\alice`.
    Wsl { distro: String, path: PathBuf },
    /// An in-panel full-text search query.
    Search(String),
    /// An entry inside an archive file.
    Archive {
        /// Path to the archive on the local file system.
        host: PathBuf,
        /// Virtual path inside the archive.
        inner: PathBuf,
    },
    /// A cloud-provider path.
    Cloud { provider: String, path: String },
    /// A virtual shell folder (e.g. Recycle Bin, Network).
    Virtual(String),
}

impl Location {
    /// Returns the discriminant of this location.
    pub fn kind(&self) -> LocationKind {
        match self {
            Location::Local(_) => LocationKind::Local,
            Location::Unc { .. } => LocationKind::Unc,
            Location::Wsl { .. } => LocationKind::Wsl,
            Location::Search(_) => LocationKind::Search,
            Location::Archive { .. } => LocationKind::Archive,
            Location::Cloud { .. } => LocationKind::Cloud,
            Location::Virtual(_) => LocationKind::Virtual,
        }
    }

    /// Parse a string into a `Location`.
    ///
    /// Rules (first match wins):
    /// - `\\wsl$\…` or `\\wsl.localhost\…` → `Wsl`
    /// - `\\server\share\…` → `Unc`
    /// - Absolute path starting with a drive letter or `/` → `Local`
    /// - Otherwise → `Local(PathBuf::from(input))`
    pub fn parse(input: &str) -> Location {
        // Normalise backslash vs forward-slash for prefix checks.
        let normalised = input.replace('/', "\\");

        if let Some(without_prefix) = normalised.strip_prefix("\\\\") {
            // strip leading \\
            let lower = without_prefix.to_ascii_lowercase();

            if lower.starts_with("wsl$\\") || lower.starts_with("wsl.localhost\\") {
                // Determine where the distro name ends.
                let skip = if lower.starts_with("wsl$\\") { 5 } else { 14 };
                let rest = &without_prefix[skip..];
                let (distro, remainder) = match rest.find('\\') {
                    Some(pos) => (&rest[..pos], &rest[pos..]),
                    None => (rest, ""),
                };
                return Location::Wsl {
                    distro: distro.to_owned(),
                    path: PathBuf::from(if remainder.is_empty() {
                        "\\"
                    } else {
                        remainder
                    }),
                };
            }

            // UNC: \\server\share\...
            let parts: Vec<&str> = without_prefix.splitn(3, '\\').collect();
            let server = parts.first().copied().unwrap_or("").to_owned();
            let share = parts.get(1).copied().unwrap_or("").to_owned();
            let path_part = parts.get(2).copied().unwrap_or("");
            return Location::Unc {
                server,
                share,
                path: PathBuf::from(if path_part.is_empty() {
                    "\\"
                } else {
                    path_part
                }),
            };
        }

        // Absolute local: drive letter (C:\) or Unix-style root (/).
        if (input.len() >= 3
            && input.as_bytes()[1] == b':'
            && (input.as_bytes()[2] == b'\\' || input.as_bytes()[2] == b'/'))
            || input.starts_with('/')
        {
            return Location::Local(PathBuf::from(input));
        }

        // Fall back to treating the string as a local path.
        Location::Local(PathBuf::from(input))
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Local(p) => write!(f, "{}", p.display()),
            Location::Unc {
                server,
                share,
                path,
            } => {
                write!(f, "\\\\{}\\{}", server, share)?;
                let s = path.to_string_lossy();
                if s != "\\" {
                    write!(f, "\\{}", s.trim_start_matches('\\'))?;
                }
                Ok(())
            }
            Location::Wsl { distro, path } => {
                write!(f, "\\\\wsl$\\{}", distro)?;
                let s = path.to_string_lossy();
                if s != "\\" {
                    write!(f, "{}", s)?;
                }
                Ok(())
            }
            Location::Search(q) => write!(f, "search:{q}"),
            Location::Archive { host, inner } => {
                write!(f, "{}!{}", host.display(), inner.display())
            }
            Location::Cloud { provider, path } => write!(f, "{provider}://{path}"),
            Location::Virtual(name) => write!(f, "virtual:{name}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unc() {
        let loc = Location::parse(r"\\server\share\path\to\dir");
        assert_eq!(
            loc,
            Location::Unc {
                server: "server".to_owned(),
                share: "share".to_owned(),
                path: PathBuf::from("path\\to\\dir"),
            }
        );
        assert_eq!(loc.kind(), LocationKind::Unc);
    }

    #[test]
    fn parse_wsl_dollar() {
        let loc = Location::parse(r"\\wsl$\Ubuntu\home\alice");
        assert_eq!(
            loc,
            Location::Wsl {
                distro: "Ubuntu".to_owned(),
                path: PathBuf::from("\\home\\alice"),
            }
        );
        assert_eq!(loc.kind(), LocationKind::Wsl);
    }

    #[test]
    fn parse_wsl_localhost() {
        let loc = Location::parse(r"\\wsl.localhost\Debian\etc");
        assert_eq!(
            loc,
            Location::Wsl {
                distro: "Debian".to_owned(),
                path: PathBuf::from("\\etc"),
            }
        );
    }

    #[test]
    fn parse_local_drive() {
        let loc = Location::parse(r"C:\Users\alice");
        assert_eq!(loc, Location::Local(PathBuf::from(r"C:\Users\alice")));
        assert_eq!(loc.kind(), LocationKind::Local);
    }

    #[test]
    fn parse_unix_root() {
        let loc = Location::parse("/home/alice");
        assert_eq!(loc, Location::Local(PathBuf::from("/home/alice")));
    }

    #[test]
    fn display_local() {
        let loc = Location::Local(PathBuf::from(r"C:\Users"));
        assert_eq!(loc.to_string(), r"C:\Users");
    }

    #[test]
    fn display_unc() {
        let loc = Location::Unc {
            server: "srv".to_owned(),
            share: "share".to_owned(),
            path: PathBuf::from("dir"),
        };
        assert_eq!(loc.to_string(), r"\\srv\share\dir");
    }

    #[test]
    fn display_wsl() {
        let loc = Location::Wsl {
            distro: "Ubuntu".to_owned(),
            path: PathBuf::from("\\home\\alice"),
        };
        assert!(loc.to_string().starts_with(r"\\wsl$\Ubuntu"));
    }

    #[test]
    fn kind_variants() {
        assert_eq!(Location::Search("q".into()).kind(), LocationKind::Search);
        assert_eq!(
            Location::Archive {
                host: PathBuf::from("a.zip"),
                inner: PathBuf::from("b"),
            }
            .kind(),
            LocationKind::Archive
        );
        assert_eq!(
            Location::Cloud {
                provider: "onedrive".into(),
                path: "/docs".into(),
            }
            .kind(),
            LocationKind::Cloud
        );
        assert_eq!(
            Location::Virtual("RecycleBin".into()).kind(),
            LocationKind::Virtual
        );
    }
}
