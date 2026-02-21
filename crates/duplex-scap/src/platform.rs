cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod windows;
        pub(crate) use windows::WindowsCapturer as PlatformCapturer;
    } else if #[cfg(target_os = "linux")] {
        pub mod linux;
        pub(crate) use linux::LinuxCapturer as PlatformCapturer;
    } else if #[cfg(target_os = "macos")] {
        pub mod macos;
        pub(crate) use macos::MacOSCapturer as PlatformCapturer;
    } else {
        mod unsupported;
        pub(crate) use unsupported::UnsupportedCapturer as PlatformCapturer;
    }
}
