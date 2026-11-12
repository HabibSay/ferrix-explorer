//! Slice-0 spike: thumbnail fetching — Tranche 3 decision 6 evidence.
//!
//! Compares `IShellItemImageFactory::GetImage` (candidate consumer for the list)
//! against `IThumbnailCache::GetThumbnail` (the Shell's own cache), cold and
//! warm, and discriminates a real thumbnail from a plain file-type icon by
//! pixel-folding the bitmaps. Prints `FINDING |` lines for
//! `docs/tranche-3-validation.md`.
//!
//! Usage: `cargo run -p fx-shell --example spike_thumbnails -- [image-path]`

use std::{os::windows::ffi::OsStrExt, path::PathBuf, time::Instant};

use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{
            BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, HBITMAP,
            HGDIOBJ, ReleaseDC,
        },
        System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoInitializeEx},
        UI::Shell::{
            ISharedBitmap, IShellItem, IShellItemImageFactory, IThumbnailCache,
            SHCreateItemFromParsingName, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, WTS_FLAGS,
        },
    },
    core::{GUID, Interface, PCWSTR},
};

/// `CLSID_LocalThumbnailCache` from `thumbcache.h` (the coclass hosting
/// `IThumbnailCache`); the windows crate does not export it. Note:
/// `e357fccd-a995-4576-b01f-234630154e96` is the interface IID, not a CLSID.
const CLSID_THUMBNAIL_CACHE: GUID = GUID::from_values(
    0x50EF_4544,
    0xAC9F,
    0x4A8E,
    [0xB2, 0x1B, 0x8A, 0x26, 0x18, 0x0D, 0xB1, 0x3F],
);

fn main() {
    let image = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(find_wallpaper);
    let Some(image) = image else {
        eprintln!(
            "usage: spike_thumbnails <image-path>   (no argument: first .jpg under C:\\Windows\\Web\\Wallpaper)"
        );
        std::process::exit(2);
    };
    println!("spike_thumbnails image={}", image.display());

    // SAFETY: one COM initialization for this thread; all shell calls below run here.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    println!("CoInitializeEx: {hr:?}");

    let item = match shell_item(&image) {
        Ok(i) => i,
        Err(e) => {
            println!("FINDING | shell_item failed: {e:?}");
            return;
        }
    };

    // --- API A: IShellItemImageFactory ------------------------------------
    let factory: IShellItemImageFactory = match item.cast() {
        Ok(f) => f,
        Err(e) => {
            println!("FINDING | cast to IShellItemImageFactory failed: {e:?}");
            return;
        }
    };

    let requested = SIZE { cx: 96, cy: 96 };
    // SAFETY: live factory and request size; returned HBITMAP is caller-owned.
    match unsafe { factory.GetImage(requested, SIIGBF_RESIZETOFIT) } {
        Ok(hbm) => {
            let cold = fold(hbm);
            // SAFETY: HBITMAP came from GetImage, which transfers ownership.
            let _ = unsafe { DeleteObject(HGDIOBJ(hbm.0)) };
            let t0 = Instant::now();
            // SAFETY: same live factory; second call should hit the Shell cache.
            let warm = unsafe { factory.GetImage(requested, SIIGBF_RESIZETOFIT) }.ok();
            let warm_ms = t0.elapsed().as_millis();
            let warm_info = warm.map(|h| {
                let f = fold(h);
                // SAFETY: caller-owned bitmap from GetImage.
                let _ = unsafe { DeleteObject(HGDIOBJ(h.0)) };
                f
            });
            println!("  factory cold: {cold:?}  warm: {warm_info:?} ({warm_ms}ms)");
        }
        Err(e) => println!("  factory GetImage cold failed: {e:?}"),
    }

    // Icon-only variant: if it folds differently from the plain request, the
    // plain request was a real thumbnail, not the file-type icon.
    // SAFETY: live factory; caller-owned result.
    match unsafe { factory.GetImage(requested, SIIGBF_ICONONLY | SIIGBF_RESIZETOFIT) } {
        Ok(hbm) => {
            let icon = fold(hbm);
            // SAFETY: caller-owned bitmap from GetImage.
            let _ = unsafe { DeleteObject(HGDIOBJ(hbm.0)) };
            println!("  factory icon-only: {icon:?}");
        }
        Err(e) => println!("  factory icon-only failed: {e:?}"),
    }

    // --- API B: IThumbnailCache -------------------------------------------
    // SAFETY: CLSID is a compile-time constant from shobjidl.h; COM is initialized.
    let cache: IThumbnailCache = match unsafe {
        windows::Win32::System::Com::CoCreateInstance(&CLSID_THUMBNAIL_CACHE, None, CLSCTX_ALL)
    } {
        Ok(c) => c,
        Err(e) => {
            println!("FINDING | CoCreateInstance(IThumbnailCache) failed: {e:?}");
            return;
        }
    };

    let mut shared: Option<ISharedBitmap> = None;
    // SAFETY: live cache and item; out-parameters are local and valid.
    let cold =
        unsafe { cache.GetThumbnail(&item, 96, WTS_FLAGS(0), Some(&mut shared), None, None) };
    match cold {
        Ok(()) => {
            let size = shared.as_ref().and_then(|s| unsafe { s.GetSize().ok() });
            println!("  thumbnail-cache cold: size={size:?}");
            let t0 = Instant::now();
            let mut shared2: Option<ISharedBitmap> = None;
            // SAFETY: same live cache; second call should hit its own cache.
            let warm = unsafe {
                cache.GetThumbnail(&item, 96, WTS_FLAGS(0), Some(&mut shared2), None, None)
            };
            let warm_ms = t0.elapsed().as_millis();
            let warm_size = shared2.as_ref().and_then(|s| unsafe { s.GetSize().ok() });
            println!("  thumbnail-cache warm: {warm:?} size={warm_size:?} ({warm_ms}ms)");
        }
        Err(e) => println!("  thumbnail-cache GetThumbnail failed: {e:?}"),
    }

    // --- Non-image control -------------------------------------------------
    if let Some(control) = control_path()
        && let Ok(txt) = shell_item(&control)
        && let Ok(f) = txt.cast::<IShellItemImageFactory>()
    {
        // SAFETY: live factory; caller-owned result.
        match unsafe { f.GetImage(requested, SIIGBF_RESIZETOFIT) } {
            Ok(hbm) => {
                let info = fold(hbm);
                // SAFETY: caller-owned bitmap from GetImage.
                let _ = unsafe { DeleteObject(HGDIOBJ(hbm.0)) };
                println!("  control (.txt) GetImage: {info:?}");
            }
            Err(e) => println!("  control GetImage failed: {e:?}"),
        }
    }

    // SAFETY: pairs with the CoInitializeEx above on this thread.
    unsafe { windows::Win32::System::Com::CoUninitialize() };
}

// ---------------------------------------------------------------- helpers

fn shell_item(path: &std::path::Path) -> windows::core::Result<IShellItem> {
    let w: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `w` stays alive for the call; the callee copies what it needs.
    unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
}

/// Reads bitmap dimensions and folds every byte into a checksum so two bitmaps
/// can be compared without keeping them alive. `None` when GDI refuses.
fn fold(hbm: HBITMAP) -> Option<(u32, u32, u64)> {
    // SAFETY: screen DC for DIB conversion; released on every return path.
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;

        // First call with no bits: fills width/height into the header.
        let got = GetDIBits(hdc, hbm, 0, 0, None, &mut info, DIB_RGB_COLORS);
        let w = info.bmiHeader.biWidth;
        let h = info.bmiHeader.biHeight.abs();
        if got == 0 || w <= 0 || h <= 0 || w > 4096 || h > 4096 {
            ReleaseDC(None, hdc);
            return None;
        }

        let mut bits = vec![0u8; (w * h * 4) as usize];
        let got2 = GetDIBits(
            hdc,
            hbm,
            0,
            h as u32,
            Some(bits.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);
        if got2 == 0 {
            return None;
        }

        let mut acc: u64 = 0x9E37_79B9_7F4A_7C15;
        for (i, b) in bits.iter().enumerate() {
            acc = acc
                .rotate_left(5)
                .wrapping_add(*b as u64)
                .wrapping_mul((i as u64) | 1);
        }
        Some((w as u32, h as u32, acc))
    }
}

fn find_wallpaper() -> Option<PathBuf> {
    let roots = [r"C:\Windows\Web\Wallpaper", r"C:\Windows\Web\4K"];
    for root in roots {
        if let Ok(mut stack) = std::fs::read_dir(root) {
            while let Some(Ok(entry)) = stack.next() {
                let path = entry.path();
                if path.is_dir() {
                    if let Ok(mut inner) = std::fs::read_dir(&path) {
                        while let Some(Ok(e2)) = inner.next() {
                            let p2 = e2.path();
                            if p2.extension().is_some_and(|x| {
                                let l = x.to_string_lossy().to_lowercase();
                                l == "jpg" || l == "jpeg" || l == "png"
                            }) {
                                return Some(p2);
                            }
                        }
                    }
                } else if path.extension().is_some_and(|x| {
                    let l = x.to_string_lossy().to_lowercase();
                    l == "jpg" || l == "jpeg" || l == "png"
                }) {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn control_path() -> Option<PathBuf> {
    let p = std::env::temp_dir().join(format!("fx-spike-thumb-control-{}.txt", std::process::id()));
    if let Err(e) = std::fs::write(&p, b"not an image") {
        println!(
            "FINDING | control_path write failed: {e:?} (path={})",
            p.display()
        );
        return None;
    }
    Some(p)
}
