cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod windows;
        pub(crate) use windows::InputInjector as PlatformInjector;
    } else if #[cfg(target_os = "linux")] {
        mod linux;
        pub(crate) use linux::InputInjector as PlatformInjector;
    } else if #[cfg(target_os = "macos")] {
        mod macos;
        pub(crate) use macos::InputInjector as PlatformInjector;
    } else {
        mod unsupported;
        pub(crate) use unsupported::InputInjector as PlatformInjector;
    }
}
