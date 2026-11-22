use crate::location::Location;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarKind {
    Home,
    Pinned,
    Recent,
    KnownFolder,
    ThisPc,
    Volume,
    Network,
    Wsl,
    DevDrive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SidebarEntry {
    pub id: String,
    pub kind: SidebarKind,
    pub label: String,
    pub location: Option<Location>,
    pub unavailable: bool,
    pub used_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

impl SidebarEntry {
    pub fn pinned(label: String, path: PathBuf) -> Self {
        Self {
            id: format!("pinned:{}", path.to_string_lossy()),
            kind: SidebarKind::Pinned,
            label,
            location: Some(Location::Local(path)),
            unavailable: false,
            used_bytes: None,
            total_bytes: None,
        }
    }

    pub fn volume(
        label: String,
        path: Option<PathBuf>,
        used_bytes: Option<u64>,
        total_bytes: Option<u64>,
        unavailable: bool,
    ) -> Self {
        Self {
            id: format!(
                "volume:{}",
                path.as_ref()
                    .map(|p| p.to_string_lossy())
                    .unwrap_or_default()
            ),
            kind: SidebarKind::Volume,
            label,
            location: path.map(Location::Local),
            unavailable,
            used_bytes,
            total_bytes,
        }
    }
}

pub fn gauge_percent(used: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    let percentage = used.saturating_mul(100) / total;
    u32::try_from(percentage.min(100)).unwrap_or(100)
}

pub fn gauge_percent_entry(entry: &SidebarEntry) -> u32 {
    match (entry.used_bytes, entry.total_bytes) {
        (Some(used), Some(total)) if !entry.unavailable => gauge_percent(used, total),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn gauge_percent_is_bounded() {
        assert_eq!(gauge_percent(25, 100), 25);
        assert_eq!(gauge_percent(150, 100), 100);
        assert_eq!(gauge_percent(25, 0), 0);
    }

    #[test]
    fn pinned_entry_is_immutable_data() {
        let entry = SidebarEntry::pinned("Documents".into(), PathBuf::from("C:\\Users\\Documents"));
        assert_eq!(entry.kind, SidebarKind::Pinned);
        assert_eq!(entry.label, "Documents");
        assert!(!entry.unavailable);
    }

    #[test]
    fn unavailable_volume_keeps_zero_capacity() {
        let entry = SidebarEntry::volume("USB".into(), None, None, None, true);
        assert!(entry.unavailable);
        assert_eq!(gauge_percent_entry(&entry), 0);
    }
}
