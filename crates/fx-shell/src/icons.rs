//! System icon cache backed by SHGetFileInfo.
//!
//! Keyed by (extension_lowercase, file_attributes_u32). Bounded LRU with
//! DestroyIcon on eviction. Lookups run off the UI thread; a lookup that fails
//! or times out yields the generic icon key 0.

use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
    SHGetFileInfoW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
use windows::core::PCWSTR;

const MAX_CACHE_ENTRIES: usize = 512;

/// Opaque handle to a cached icon. Key 0 means "generic / unknown".
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IconKey(pub u64);

/// LRU icon cache. NOT thread-safe; call only from the background icon-fetching thread.
pub struct IconCache {
    /// Map from (extension_lowercase, attributes) to (IconKey, HICON).
    entries: std::collections::HashMap<(String, u32), (IconKey, HICON)>,
    /// Insertion-order list of cache keys for LRU eviction.
    order: Vec<(String, u32)>,
    next_key: u64,
}

impl IconCache {
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            order: Vec::new(),
            next_key: 1,
        }
    }

    /// Look up or fetch the icon for a file with the given extension and attributes.
    ///
    /// `extension` should be lowercase, e.g. `".txt"`. Use an empty string for directories.
    /// Returns `IconKey(0)` on failure.
    pub fn get_or_fetch(&mut self, extension: &str, attributes: u32, small: bool) -> IconKey {
        let cache_key = (extension.to_ascii_lowercase(), attributes);
        if let Some((key, _hicon)) = self.entries.get(&cache_key) {
            return *key;
        }
        // Fetch from Shell and insert on success.
        match self.fetch_from_shell(extension, attributes, small) {
            Some((icon_key, hicon)) => {
                self.insert(cache_key, icon_key, hicon);
                icon_key
            }
            None => IconKey(0),
        }
    }

    /// Call SHGetFileInfo to obtain an HICON for the given extension / attributes.
    ///
    /// Returns `None` on failure (null handle, error return, etc.).
    fn fetch_from_shell(
        &mut self,
        extension: &str,
        attributes: u32,
        small: bool,
    ) -> Option<(IconKey, HICON)> {
        // Build a fake filename with the extension so SHGetFileInfo can look up
        // the icon by extension type without touching the disk.
        let fake_name = format!("file{extension}\0");
        let wide: Vec<u16> = fake_name.encode_utf16().collect();

        let mut info = SHFILEINFOW::default();
        let flags = SHGFI_ICON
            | SHGFI_USEFILEATTRIBUTES
            | if small {
                SHGFI_SMALLICON
            } else {
                SHGFI_LARGEICON
            };

        // SAFETY: `wide` is a valid NUL-terminated UTF-16 string kept alive for the
        // duration of this call; `info` is a writable output struct of the correct size.
        let result = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                FILE_FLAGS_AND_ATTRIBUTES(attributes),
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };

        if result == 0 || info.hIcon.is_invalid() {
            return None;
        }

        let key = IconKey(self.next_key);
        self.next_key += 1;
        Some((key, info.hIcon))
    }

    /// Insert a new entry, evicting the oldest if the cache is at capacity.
    fn insert(&mut self, cache_key: (String, u32), icon_key: IconKey, hicon: HICON) {
        if self.order.len() >= MAX_CACHE_ENTRIES {
            // Evict the oldest entry (LRU = front of insertion-order list).
            let oldest = self.order.remove(0);
            if let Some((_k, evicted_hicon)) = self.entries.remove(&oldest) {
                // SAFETY: `evicted_hicon` was obtained from SHGetFileInfo and is owned
                // exclusively by this cache; no other code holds a copy of this handle.
                unsafe {
                    let _ = DestroyIcon(evicted_hicon);
                }
            }
        }
        self.entries.insert(cache_key.clone(), (icon_key, hicon));
        self.order.push(cache_key);
    }

    /// Destroy all cached icons. Called automatically on `Drop`.
    pub fn clear(&mut self) {
        for (_k, (_key, hicon)) in self.entries.drain() {
            // SAFETY: All HICONs in the map were obtained from SHGetFileInfo and are
            // owned exclusively by this cache; clearing releases every handle exactly once.
            unsafe {
                let _ = DestroyIcon(hicon);
            }
        }
        self.order.clear();
        self.next_key = 1;
    }
}

impl Drop for IconCache {
    fn drop(&mut self) {
        self.clear();
    }
}

impl Default for IconCache {
    fn default() -> Self {
        Self::new()
    }
}
