//! A shared-memory region that the engine process opens by name.
//!
//! POSIX shared memory (`shm_open`) on Unix, a named file mapping on Windows. Android has no POSIX shared memory, so
//! there the region is an ordinary file in the temporary directory, mapped shared -- the engine still opens it by name. The region is split in two halves: the
//! input area, written by the host and read by the engine, and the output area, written by the engine and read by
//! the host. The protocol is synchronous (`engine/HOST.md`): the engine touches the region only between a request and
//! its response, and the host holds `&mut Engine` for that whole time, so no Rust reference into the region is alive
//! while the other process writes to it.
#![allow(unsafe_code)]

use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Opens (or with `O_CREAT`, creates) the object behind a region's name.
#[cfg(all(unix, not(target_os = "android")))]
unsafe fn open_region(name: *const libc::c_char, flags: libc::c_int) -> libc::c_int {
    // SAFETY: the caller passes a valid NUL-terminated name.
    unsafe { libc::shm_open(name, flags, 0o600) }
}

#[cfg(target_os = "android")]
unsafe fn open_region(name: *const libc::c_char, flags: libc::c_int) -> libc::c_int {
    // SAFETY: the caller passes a valid NUL-terminated path.
    unsafe { libc::open(name, flags, 0o600 as libc::c_uint) }
}

/// Removes a region's name; the mappings stay valid.
#[cfg(all(unix, not(target_os = "android")))]
unsafe fn unlink_region(name: *const libc::c_char) -> libc::c_int {
    // SAFETY: the caller passes a valid NUL-terminated name.
    unsafe { libc::shm_unlink(name) }
}

#[cfg(target_os = "android")]
unsafe fn unlink_region(name: *const libc::c_char) -> libc::c_int {
    // SAFETY: the caller passes a valid NUL-terminated path.
    unsafe { libc::unlink(name) }
}

/// A region the engine opens by its name.
#[derive(Debug)]
pub struct SharedMemory {
    name: String,
    ptr: *mut u8,
    len: usize,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the region is plain memory owned by this value; access goes through `&self`/`&mut self`.
unsafe impl Send for SharedMemory {}

impl SharedMemory {
    /// Creates a region of at least `len` bytes, rounded up to whole pages.
    ///
    /// # Errors
    /// The operating system refused to create or map the region.
    pub fn create(len: usize) -> io::Result<Self> {
        let len = len.max(1).div_ceil(1 << 16) << 16;
        // a name left behind by a process that crashed before unlinking it is skipped
        let mut attempts = 0;
        loop {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            match Self::create_named(&Self::name_for(std::process::id(), id), len) {
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 16 => attempts += 1,
                result => return result,
            }
        }
    }

    #[cfg(all(unix, not(target_os = "android")))]
    fn name_for(pid: u32, id: u32) -> String {
        // macOS allows 31 characters
        format!("/bk-{pid}-{id}")
    }

    #[cfg(target_os = "android")]
    fn name_for(pid: u32, id: u32) -> String {
        std::env::temp_dir().join(format!("omnarc-{pid}-{id}")).to_string_lossy().into_owned()
    }

    #[cfg(windows)]
    fn name_for(pid: u32, id: u32) -> String {
        format!("Local\\omnarc-{pid}-{id}")
    }

    #[cfg(unix)]
    fn create_named(name: &str, len: usize) -> io::Result<Self> {
        let cname = std::ffi::CString::new(name).map_err(io::Error::other)?;
        let size = libc::off_t::try_from(len).map_err(io::Error::other)?;
        // SAFETY: cname is a valid NUL-terminated string; O_EXCL makes sure the region is new.
        let fd = unsafe { open_region(cname.as_ptr(), libc::O_CREAT | libc::O_EXCL | libc::O_RDWR) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is the descriptor opened above; mapping a fresh region of `len` bytes after sizing it.
        let ptr = unsafe {
            if libc::ftruncate(fd, size) != 0 {
                let error = io::Error::last_os_error();
                libc::close(fd);
                unlink_region(cname.as_ptr());
                return Err(error);
            }
            let ptr =
                libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
            libc::close(fd);
            if ptr == libc::MAP_FAILED {
                let error = io::Error::last_os_error();
                unlink_region(cname.as_ptr());
                return Err(error);
            }
            ptr.cast::<u8>()
        };
        Ok(Self { name: name.to_owned(), ptr, len })
    }

    #[cfg(windows)]
    fn create_named(name: &str, len: usize) -> io::Result<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE,
        };

        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let size = len as u64;
        // SAFETY: a pagefile-backed mapping with a NUL-terminated wide name; the view covers the whole mapping.
        unsafe {
            #[allow(clippy::cast_possible_truncation)] // the size is split into its two halves
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                (size >> 32) as u32,
                size as u32,
                wide.as_ptr(),
            );
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len);
            if view.Value.is_null() {
                let error = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(error);
            }
            Ok(Self { name: name.to_owned(), ptr: view.Value.cast::<u8>(), len, handle })
        }
    }

    /// The name the engine opens the region by.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Size of each half.
    #[must_use]
    pub fn half(&self) -> usize {
        self.len / 2
    }

    /// Removes the name once the engine has opened the region, so nothing is left behind if either process dies.
    /// The mappings stay valid. On Windows the name disappears with the last handle, so this does nothing.
    #[cfg_attr(windows, allow(clippy::unused_self))]
    pub fn unlink(&self) {
        #[cfg(unix)]
        if let Ok(cname) = std::ffi::CString::new(self.name.as_str()) {
            // SAFETY: a valid NUL-terminated name; unlinking an already removed name only returns an error.
            unsafe {
                unlink_region(cname.as_ptr());
            }
        }
    }

    /// The input area, written by the host.
    pub fn input_mut(&mut self) -> &mut [u8] {
        // SAFETY: ptr..ptr+len is mapped for the lifetime of self; the engine does not touch the region while the
        // host holds `&mut self` outside a request (see the module documentation).
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.half()) }
    }

    /// The output area, written by the engine during a request.
    #[must_use]
    pub fn output(&self) -> &[u8] {
        // SAFETY: as for input_mut; the second half starts at a page-aligned offset inside the mapping.
        unsafe { std::slice::from_raw_parts(self.ptr.add(self.half()), self.half()) }
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            self.unlink();
            // SAFETY: unmapping exactly the range mapped in create_named; no references outlive self.
            unsafe {
                libc::munmap(self.ptr.cast(), self.len);
            }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
            // SAFETY: unmapping the view and closing the mapping created in create_named.
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.ptr.cast() });
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SharedMemory;

    #[test]
    fn halves_are_separate_and_page_aligned() {
        let mut shm = SharedMemory::create(10_000).unwrap();
        assert!(shm.len() >= 10_000);
        assert_eq!(shm.half() % 4096, 0);
        shm.input_mut()[0] = 7;
        assert_eq!(shm.output()[0], 0);
        assert_eq!(shm.input_mut().len(), shm.output().len());
    }
}
