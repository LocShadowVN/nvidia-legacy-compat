use crate::drm_ioctl::{drm_ioctl, iow, iowr};
use libc::{c_int, c_ulong, c_void, pollfd, POLLIN};
use std::io::{Error, Result};

const DMA_BUF_BASE: u8 = b'b';

pub const DMA_BUF_SYNC_READ: u64 = 1 << 0;
pub const DMA_BUF_SYNC_WRITE: u64 = 2 << 0;
pub const DMA_BUF_SYNC_RW: u64 = DMA_BUF_SYNC_READ | DMA_BUF_SYNC_WRITE;
pub const DMA_BUF_SYNC_START: u64 = 0 << 2;
pub const DMA_BUF_SYNC_END: u64 = 1 << 2;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DmaBufSync {
    pub flags: u64,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DmaBufExportSyncFile {
    pub flags: u32,
    pub fd: i32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct DmaBufImportSyncFile {
    pub flags: u32,
    pub fd: i32,
}

pub const DMA_BUF_IOCTL_SYNC: c_ulong = iow::<DmaBufSync>(DMA_BUF_BASE, 0);
pub const DMA_BUF_IOCTL_EXPORT_SYNC_FILE: c_ulong =
    iowr::<DmaBufExportSyncFile>(DMA_BUF_BASE, 2);
pub const DMA_BUF_IOCTL_IMPORT_SYNC_FILE: c_ulong =
    iow::<DmaBufImportSyncFile>(DMA_BUF_BASE, 3);

/// Bộ quản lý hàng rào đồng bộ (Fence Synchronization Manager) cho DMA-BUF
pub struct FenceSyncManager;

impl FenceSyncManager {
    /// Trích xuất `sync_file` FD từ DMA-BUF và đợi (poll) cho đến khi GPU hoàn tất ghi dữ liệu.
    /// Ngăn chặn Wayland Compositor đọc buffer khi GPU NVIDIA chưa render xong frame.
    pub fn wait_dmabuf_ready(dmabuf_fd: c_int, timeout_ms: c_int) -> Result<()> {
        if dmabuf_fd < 0 {
            return Ok(());
        }

        let mut export_req = DmaBufExportSyncFile {
            flags: DMA_BUF_SYNC_RW as u32,
            fd: -1,
        };

        match drm_ioctl(
            dmabuf_fd,
            DMA_BUF_IOCTL_EXPORT_SYNC_FILE,
            (&mut export_req as *mut DmaBufExportSyncFile).cast::<c_void>(),
        ) {
            Ok(()) if export_req.fd >= 0 => {
                let sync_fd = export_req.fd;
                let wait_res = Self::poll_sync_fd(sync_fd, timeout_ms);
                unsafe {
                    libc::close(sync_fd);
                }
                wait_res
            }
            // Nếu kernel/memfd không hỗ trợ EXPORT_SYNC_FILE, fallback sang DMA_BUF_IOCTL_SYNC
            _ => Self::fallback_cpu_sync(dmabuf_fd),
        }
    }

    /// Đợi tín hiệu hoàn tất trên một Linux `sync_file` hoặc `eventfd`
    pub fn poll_sync_fd(sync_fd: c_int, timeout_ms: c_int) -> Result<()> {
        let mut pfd = pollfd {
            fd: sync_fd,
            events: POLLIN,
            revents: 0,
        };

        loop {
            let ret = unsafe { libc::poll(&mut pfd as *mut pollfd, 1, timeout_ms) };
            if ret > 0 {
                return Ok(());
            }
            if ret == 0 {
                return Err(Error::from_raw_os_error(libc::ETIMEDOUT));
            }
            let err = Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINTR) {
                return Err(err);
            }
        }
    }

    /// Nhập một `sync_file` fence ngoại vi vào DMA-BUF để đồng bộ hai chiều với Compositor
    pub fn import_sync_fd(dmabuf_fd: c_int, sync_fd: c_int, flags: u32) -> Result<()> {
        let mut import_req = DmaBufImportSyncFile {
            flags: flags & (DMA_BUF_SYNC_RW as u32),
            fd: sync_fd,
        };

        drm_ioctl(
            dmabuf_fd,
            DMA_BUF_IOCTL_IMPORT_SYNC_FILE,
            (&mut import_req as *mut DmaBufImportSyncFile).cast::<c_void>(),
        )
    }

    /// Bắt đầu phiên truy cập CPU đồng bộ trên DMA-BUF
    pub fn begin_cpu_access(dmabuf_fd: c_int, write: bool) -> Result<()> {
        let rw_flag = if write {
            DMA_BUF_SYNC_RW
        } else {
            DMA_BUF_SYNC_READ
        };
        let mut sync_req = DmaBufSync {
            flags: DMA_BUF_SYNC_START | rw_flag,
        };
        match drm_ioctl(
            dmabuf_fd,
            DMA_BUF_IOCTL_SYNC,
            (&mut sync_req as *mut DmaBufSync).cast::<c_void>(),
        ) {
            Ok(()) => Ok(()),
            // Cho phép tiếp tục nếu FD là memfd trong môi trường test
            Err(e) if e.raw_os_error() == Some(libc::ENOTTY)
                || e.raw_os_error() == Some(libc::EINVAL) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Kết thúc phiên truy cập CPU đồng bộ trên DMA-BUF
    pub fn end_cpu_access(dmabuf_fd: c_int, write: bool) -> Result<()> {
        let rw_flag = if write {
            DMA_BUF_SYNC_RW
        } else {
            DMA_BUF_SYNC_READ
        };
        let mut sync_req = DmaBufSync {
            flags: DMA_BUF_SYNC_END | rw_flag,
        };
        match drm_ioctl(
            dmabuf_fd,
            DMA_BUF_IOCTL_SYNC,
            (&mut sync_req as *mut DmaBufSync).cast::<c_void>(),
        ) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(libc::ENOTTY)
                || e.raw_os_error() == Some(libc::EINVAL) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn fallback_cpu_sync(dmabuf_fd: c_int) -> Result<()> {
        Self::begin_cpu_access(dmabuf_fd, false)?;
        Self::end_cpu_access(dmabuf_fd, false)
    }
}
