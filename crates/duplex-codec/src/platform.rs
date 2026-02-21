cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod windows;
        pub(crate) use windows::{PlatformVideoDecoder, PlatformVideoEncoder};
    } else if #[cfg(target_os = "linux")] {
        mod linux;
        pub(crate) use linux::{PlatformVideoDecoder, PlatformVideoEncoder};
    } else if #[cfg(target_os = "macos")] {
        mod macos;
        pub(crate) use macos::{PlatformVideoDecoder, PlatformVideoEncoder};
    } else {
        mod unsupported;
        pub(crate) use unsupported::{PlatformVideoDecoder, PlatformVideoEncoder};
    }
}
