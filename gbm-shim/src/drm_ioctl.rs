use libc::{c_int, c_ulong, c_void, off_t, size_t};
use std::io::{Error, Result};
use std::ptr;

// Chuẩn Linux Kernel UAPI <drm/drm.h> & <drm/drm_mode.h>
const IOC_NRBITS: c_ulong = 8;
const IOC_TYPEBITS: c_ulong = 8;
const IOC_SIZEBITS: c_ulong = 14;

const IOC_NRSHIFT: c_ulong = 0;
const IOC_TYPESHIFT: c_ulong = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: c_ulong = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: c_ulong = IOC_SIZESHIFT + IOC_SIZEBITS;

const IOC_WRITE: c_ulong = 1;
const IOC_READ: c_ulong = 2;

pub const fn ioc(dir: c_ulong, ty: c_ulong, nr: c_ulong, size: c_ulong) -> c_ulong {
    (dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT)
}

pub const fn iow<T>(ty: u8, nr: u8) -> c_ulong {
    ioc(
        IOC_WRITE,
        ty as c_ulong,
        nr as c_ulong,
        std::mem::size_of::<T>() as c_ulong,
    )
}

pub const fn iowr<T>(ty: u8, nr: u8) -> c_ulong {
    ioc(
        IOC_READ | IOC_WRITE,
        ty as c_ulong,
        nr as c_ulong,
        std::mem::size_of::<T>() as c_ulong,
    )
}

const DRM_IOCTL_BASE: u8 = b'd';

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmModeCreateDumb {
    pub height: u32,
    pub width: u32,
    pub bpp: u32,
    pub flags: u32,
    pub handle: u32,
    pub pitch: u32,
    pub size: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmModeMapDumb {
    pub handle: u32,
    pub pad: u32,
    pub offset: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmModeDestroyDumb {
    pub handle: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmPrimeHandle {
    pub handle: u32,
    pub flags: u32,
    pub fd: i32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DrmGemClose {
    pub handle: u32,
    pub pad: u32,
}

pub const DRM_IOCTL_GEM_CLOSE: c_ulong = iow::<DrmGemClose>(DRM_IOCTL_BASE, 0x09);
pub const DRM_IOCTL_PRIME_HANDLE_TO_FD: c_ulong = iowr::<DrmPrimeHandle>(DRM_IOCTL_BASE, 0x2d);
pub const DRM_IOCTL_PRIME_FD_TO_HANDLE: c_ulong = iowr::<DrmPrimeHandle>(DRM_IOCTL_BASE, 0x2e);
pub const DRM_IOCTL_MODE_CREATE_DUMB: c_ulong = iowr::<DrmModeCreateDumb>(DRM_IOCTL_BASE, 0xb2);
pub const DRM_IOCTL_MODE_MAP_DUMB: c_ulong = iowr::<DrmModeMapDumb>(DRM_IOCTL_BASE, 0xb3);
pub const DRM_IOCTL_MODE_DESTROY_DUMB: c_ulong = iowr::<DrmModeDestroyDumb>(DRM_IOCTL_BASE, 0xb4);

pub const DRM_CLOEXEC: u32 = libc::O_CLOEXEC as u32;
pub const DRM_RDWR: u32 = libc::O_RDWR as u32;

/// Thực thi ioctl có tự động thử lại khi gặp tín hiệu ngắt (EINTR)
pub fn drm_ioctl(fd: c_int, request: c_ulong, arg: *mut c_void) -> Result<()> {
    loop {
        let ret = unsafe { libc::ioctl(fd, request, arg) };
        if ret == 0 {
            return Ok(());
        }
        let err = Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            return Err(err);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DumbBufferAllocation {
    pub handle: u32,
    pub pitch: u32,
    pub size: u64,
    pub is_mock_memfd: bool,
    pub backing_fd: c_int,
}

/// Cấp phát buffer qua DRM_IOCTL_MODE_CREATE_DUMB.
/// Hỗ trợ fallback sang memfd cho môi trường kiểm thử không có thiết bị DRM vật lý.
pub fn create_dumb_buffer(
    drm_fd: c_int,
    width: u32,
    height: u32,
    bpp: u32,
    allow_memfd_fallback: bool,
) -> Result<DumbBufferAllocation> {
    let mut req = DrmModeCreateDumb {
        width,
        height,
        bpp,
        flags: 0,
        handle: 0,
        pitch: 0,
        size: 0,
    };

    match drm_ioctl(
        drm_fd,
        DRM_IOCTL_MODE_CREATE_DUMB,
        (&mut req as *mut DrmModeCreateDumb).cast::<c_void>(),
    ) {
        Ok(()) => Ok(DumbBufferAllocation {
            handle: req.handle,
            pitch: req.pitch,
            size: req.size,
            is_mock_memfd: false,
            backing_fd: -1,
        }),
        Err(e) if allow_memfd_fallback => {
            let bytes_per_pixel = bpp.div_ceil(8);
            // Căn chỉnh stride (pitch) theo bội số của 64 bytes (chuẩn GPU cacheline)
            let raw_pitch = width.saturating_mul(bytes_per_pixel);
            let pitch = (raw_pitch + 63) & !63;
            let size = (pitch as u64).saturating_mul(height as u64);

            let name = c"nv_legacy_gbm_dumb";
            let memfd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
            if memfd < 0 {
                return Err(e);
            }
            if unsafe { libc::ftruncate(memfd, size as off_t) } < 0 {
                let trunc_err = Error::last_os_error();
                unsafe { libc::close(memfd) };
                return Err(trunc_err);
            }

            Ok(DumbBufferAllocation {
                handle: memfd as u32,
                pitch,
                size,
                is_mock_memfd: true,
                backing_fd: memfd,
            })
        }
        Err(e) => Err(e),
    }
}

/// Chuyển đổi GEM handle sang DMA-BUF File Descriptor qua DRM_IOCTL_PRIME_HANDLE_TO_FD
pub fn export_prime_fd(drm_fd: c_int, alloc: &DumbBufferAllocation) -> Result<c_int> {
    if alloc.is_mock_memfd {
        let dup_fd = unsafe { libc::fcntl(alloc.backing_fd, libc::F_DUPFD_CLOEXEC, 0) };
        if dup_fd < 0 {
            return Err(Error::last_os_error());
        }
        return Ok(dup_fd);
    }

    let mut prime = DrmPrimeHandle {
        handle: alloc.handle,
        flags: DRM_CLOEXEC | DRM_RDWR,
        fd: -1,
    };

    drm_ioctl(
        drm_fd,
        DRM_IOCTL_PRIME_HANDLE_TO_FD,
        (&mut prime as *mut DrmPrimeHandle).cast::<c_void>(),
    )?;

    Ok(prime.fd)
}

/// Nhập DMA-BUF FD thành GEM Handle qua DRM_IOCTL_PRIME_FD_TO_HANDLE
pub fn import_prime_fd(
    drm_fd: c_int,
    dmabuf_fd: c_int,
    allow_memfd_fallback: bool,
) -> Result<(u32, bool, c_int)> {
    let mut prime = DrmPrimeHandle {
        handle: 0,
        flags: 0,
        fd: dmabuf_fd,
    };

    match drm_ioctl(
        drm_fd,
        DRM_IOCTL_PRIME_FD_TO_HANDLE,
        (&mut prime as *mut DrmPrimeHandle).cast::<c_void>(),
    ) {
        Ok(()) => Ok((prime.handle, false, -1)),
        Err(e) if allow_memfd_fallback => {
            let dup_fd = unsafe { libc::fcntl(dmabuf_fd, libc::F_DUPFD_CLOEXEC, 0) };
            if dup_fd < 0 {
                return Err(e);
            }
            Ok((dup_fd as u32, true, dup_fd))
        }
        Err(e) => Err(e),
    }
}

/// Ánh xạ bộ nhớ Dumb Buffer vào không gian địa chỉ tiến trình
pub fn map_dumb_buffer(
    drm_fd: c_int,
    alloc: &DumbBufferAllocation,
    prot: c_int,
) -> Result<*mut c_void> {
    let (map_fd, offset) = if alloc.is_mock_memfd {
        (alloc.backing_fd, 0 as off_t)
    } else {
        let mut map_req = DrmModeMapDumb {
            handle: alloc.handle,
            pad: 0,
            offset: 0,
        };
        drm_ioctl(
            drm_fd,
            DRM_IOCTL_MODE_MAP_DUMB,
            (&mut map_req as *mut DrmModeMapDumb).cast::<c_void>(),
        )?;
        (drm_fd, map_req.offset as off_t)
    };

    let addr = unsafe {
        libc::mmap(
            ptr::null_mut(),
            alloc.size as size_t,
            prot,
            libc::MAP_SHARED,
            map_fd,
            offset,
        )
    };

    if addr == libc::MAP_FAILED {
        return Err(Error::last_os_error());
    }
    Ok(addr)
}

/// Giải phóng tài nguyên GEM / Dumb Buffer
pub fn destroy_dumb_buffer(drm_fd: c_int, alloc: &DumbBufferAllocation, imported: bool) {
    if alloc.is_mock_memfd {
        if alloc.backing_fd >= 0 {
            unsafe {
                libc::close(alloc.backing_fd);
            }
        }
        return;
    }

    if imported {
        let mut close_req = DrmGemClose {
            handle: alloc.handle,
            pad: 0,
        };
        let _ = drm_ioctl(
            drm_fd,
            DRM_IOCTL_GEM_CLOSE,
            (&mut close_req as *mut DrmGemClose).cast::<c_void>(),
        );
    } else {
        let mut destroy_req = DrmModeDestroyDumb {
            handle: alloc.handle,
        };
        let _ = drm_ioctl(
            drm_fd,
            DRM_IOCTL_MODE_DESTROY_DUMB,
            (&mut destroy_req as *mut DrmModeDestroyDumb).cast::<c_void>(),
        );
    }
}
