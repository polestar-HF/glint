//! Giving freed memory back to the OS after an engine is dropped.

/// Asks the allocator / OS to return freed memory: `malloc_trim` on Linux (glibc), a working-set
/// trim on Windows, nothing elsewhere. Call it after dropping a [`crate::PaddleOcr`] (idle
/// unload). Dropping the engine frees the model weights and activations, but glibc keeps the
/// pages mapped and Windows keeps them in the working set until trimmed; measured here, trimming
/// takes the process from about 85 MB to about 40 MB, of which 11 MB is private heap and the rest
/// is the shared, reclaimable ONNX Runtime library image.
pub fn trim_process_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: glibc function that takes only an integer argument.
        unsafe { malloc_trim(0) };
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        // SAFETY: pseudo-handle of the current process; (SIZE_MAX, SIZE_MAX) means "trim".
        let _ = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }
}
