#![allow(
    non_camel_case_types,
    clippy::missing_safety_doc,
    clippy::too_many_arguments
)]

pub mod drm_ioctl;
pub mod egl_shim;
pub mod sync;

use crate::drm_ioctl::{
    create_dumb_buffer, destroy_dumb_buffer, export_prime_fd, import_prime_fd, map_dumb_buffer,
    DumbBufferAllocation,
};
use crate::egl_shim::{GlReadPixelsFn, GL_BGRA_EXT, GL_UNSIGNED_BYTE};
use crate::sync::FenceSyncManager;
use libc::{c_char, c_int, c_void, size_t};
use std::ffi::CStr;
use std::ptr;
use std::sync::Mutex;

// ============================================================================
// Hằng số định dạng & cờ chuẩn GBM / DRM FourCC
// ============================================================================
const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub const GBM_FORMAT_XRGB8888: u32 = fourcc(b'X', b'R', b'2', b'4');
pub const GBM_FORMAT_ARGB8888: u32 = fourcc(b'A', b'R', b'2', b'4');
pub const GBM_FORMAT_XBGR8888: u32 = fourcc(b'X', b'B', b'2', b'4');
pub const GBM_FORMAT_ABGR8888: u32 = fourcc(b'A', b'B', b'2', b'4');
pub const GBM_FORMAT_RGB888: u32 = fourcc(b'R', b'G', b'2', b'4');
pub const GBM_FORMAT_BGR888: u32 = fourcc(b'B', b'G', b'2', b'4');
pub const GBM_FORMAT_RGB565: u32 = fourcc(b'R', b'G', b'1', b'6');
pub const GBM_FORMAT_XRGB2101010: u32 = fourcc(b'X', b'R', b'3', b'0');
pub const GBM_FORMAT_ARGB2101010: u32 = fourcc(b'A', b'R', b'3', b'0');

// Legacy GBM enum formats
pub const GBM_BO_FORMAT_XRGB8888: u32 = 0;
pub const GBM_BO_FORMAT_ARGB8888: u32 = 1;

// GBM BO usage flags
pub const GBM_BO_USE_SCANOUT: u32 = 1;
pub const GBM_BO_USE_CURSOR: u32 = 1 << 1;
pub const GBM_BO_USE_RENDERING: u32 = 1 << 2;
pub const GBM_BO_USE_WRITE: u32 = 1 << 3;
pub const GBM_BO_USE_LINEAR: u32 = 1 << 4;
pub const GBM_BO_USE_PROTECTED: u32 = 1 << 5;
pub const GBM_BO_USE_FRONT_RENDERING: u32 = 1 << 6;

// GBM Transfer flags
pub const GBM_BO_TRANSFER_READ: u32 = 1;
pub const GBM_BO_TRANSFER_WRITE: u32 = 1 << 1;
pub const GBM_BO_TRANSFER_READ_WRITE: u32 = GBM_BO_TRANSFER_READ | GBM_BO_TRANSFER_WRITE;

// GBM Import constants
pub const GBM_BO_IMPORT_WL_BUFFER: u32 = 0x5501;
pub const GBM_BO_IMPORT_EGL_IMAGE: u32 = 0x5502;
pub const GBM_BO_IMPORT_FD: u32 = 0x5503;
pub const GBM_BO_IMPORT_FD_MODIFIER: u32 = 0x5504;

pub const GBM_MAX_PLANES: usize = 4;
pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;
pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

const BACKEND_NAME: &CStr = c"nvidia-legacy-drm-gbm";
const DEFAULT_FENCE_TIMEOUT_MS: c_int = 3000;
const SURFACE_BUFFER_COUNT: usize = 3;

// ============================================================================
// Cấu trúc dữ liệu C-ABI
// ============================================================================
#[repr(C)]
#[derive(Clone, Copy)]
pub union gbm_bo_handle {
    pub ptr: *mut c_void,
    pub s32: i32,
    pub u32_: u32,
    pub s64: i64,
    pub u64_: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct gbm_import_fd_data {
    pub fd: c_int,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct gbm_import_fd_modifier_data {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub num_fds: u32,
    pub fds: [c_int; GBM_MAX_PLANES],
    pub strides: [c_int; GBM_MAX_PLANES],
    pub offsets: [c_int; GBM_MAX_PLANES],
    pub modifier: u64,
}

pub type GbmDestroyUserDataFn = Option<unsafe extern "C" fn(bo: *mut gbm_bo, data: *mut c_void)>;

struct UserDataEntry {
    data: *mut c_void,
    destroy_fn: GbmDestroyUserDataFn,
}

unsafe impl Send for UserDataEntry {}

struct MappedRegion {
    base_ptr: *mut c_void,
    length: usize,
    dmabuf_fd: c_int,
    write_access: bool,
}

unsafe impl Send for MappedRegion {}

#[repr(C)]
pub struct gbm_device {
    drm_fd: c_int,
    allow_memfd_fallback: bool,
}

#[repr(C)]
pub struct gbm_bo {
    device: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    bpp: u32,
    flags: u32,
    modifier: u64,
    offset: u32,
    alloc: DumbBufferAllocation,
    imported: bool,
    cached_dmabuf_fd: Mutex<Option<c_int>>,
    user_data: Mutex<UserDataEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Free,
    Locked,
}

struct SurfaceSlot {
    bo: *mut gbm_bo,
    state: SlotState,
}

unsafe impl Send for SurfaceSlot {}

#[repr(C)]
pub struct gbm_surface {
    pub(crate) device: *mut gbm_device,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: u32,
    pub(crate) flags: u32,
    pub(crate) modifier: u64,
    slots: Mutex<Vec<SurfaceSlot>>,
}

/// Chép dữ liệu pixel từ Pbuffer/FBO của GPU NVIDIA vào slot khả dụng của `gbm_surface`
pub(crate) unsafe fn gbm_surface_blit_pixels(
    surface: *mut gbm_surface,
    read_pixels_fn: GlReadPixelsFn,
) -> bool {
    if surface.is_null() {
        return false;
    }
    let surf = &*surface;
    let target_bo = match surf.slots.lock() {
        Ok(slots) => slots
            .iter()
            .find(|s| s.state == SlotState::Free)
            .or_else(|| slots.first())
            .map(|s| s.bo)
            .unwrap_or(ptr::null_mut()),
        Err(_) => return false,
    };

    if target_bo.is_null() {
        return false;
    }

    let mut stride: u32 = 0;
    let mut map_data: *mut c_void = ptr::null_mut();
    let mapped_ptr = gbm_bo_map(
        target_bo,
        0,
        0,
        surf.width,
        surf.height,
        GBM_BO_TRANSFER_WRITE,
        &mut stride,
        &mut map_data,
    );
    if mapped_ptr.is_null() {
        return false;
    }

    read_pixels_fn(
        0,
        0,
        surf.width as i32,
        surf.height as i32,
        GL_BGRA_EXT,
        GL_UNSIGNED_BYTE,
        mapped_ptr,
    );

    gbm_bo_unmap(target_bo, map_data);
    true
}

// ============================================================================
// Hàm nội bộ hỗ trợ định dạng pixel
// ============================================================================
fn normalize_format(format: u32) -> u32 {
    match format {
        GBM_BO_FORMAT_XRGB8888 => GBM_FORMAT_XRGB8888,
        GBM_BO_FORMAT_ARGB8888 => GBM_FORMAT_ARGB8888,
        other => other,
    }
}

fn format_to_bpp(format: u32) -> Option<u32> {
    match normalize_format(format) {
        GBM_FORMAT_XRGB8888
        | GBM_FORMAT_ARGB8888
        | GBM_FORMAT_XBGR8888
        | GBM_FORMAT_ABGR8888
        | GBM_FORMAT_XRGB2101010
        | GBM_FORMAT_ARGB2101010 => Some(32),
        GBM_FORMAT_RGB888 | GBM_FORMAT_BGR888 => Some(24),
        GBM_FORMAT_RGB565 => Some(16),
        _ => None,
    }
}

fn select_modifier(modifiers: *const u64, count: u32) -> Option<u64> {
    if modifiers.is_null() || count == 0 {
        return Some(DRM_FORMAT_MOD_LINEAR);
    }
    let slice = unsafe { std::slice::from_raw_parts(modifiers, count as usize) };
    if slice.contains(&DRM_FORMAT_MOD_LINEAR) || slice.contains(&DRM_FORMAT_MOD_INVALID) {
        Some(DRM_FORMAT_MOD_LINEAR)
    } else {
        None
    }
}

// ============================================================================
// C-ABI: Quản lý Thiết bị (gbm_device)
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn gbm_create_device(fd: c_int) -> *mut gbm_device {
    if fd < 0 {
        return ptr::null_mut();
    }

    let allow_fallback = std::env::var("NV_GBM_ALLOW_MEMFD_FALLBACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(cfg!(test));

    let dev = Box::new(gbm_device {
        drm_fd: fd,
        allow_memfd_fallback: allow_fallback,
    });
    Box::into_raw(dev)
}

#[no_mangle]
pub unsafe extern "C" fn gbm_device_get_fd(gbm: *mut gbm_device) -> c_int {
    if gbm.is_null() {
        return -1;
    }
    (*gbm).drm_fd
}

#[no_mangle]
pub unsafe extern "C" fn gbm_device_get_backend_name(gbm: *mut gbm_device) -> *const c_char {
    if gbm.is_null() {
        return ptr::null();
    }
    BACKEND_NAME.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn gbm_device_is_format_supported(
    gbm: *mut gbm_device,
    format: u32,
    flags: u32,
) -> c_int {
    if gbm.is_null() {
        return 0;
    }
    if (flags & GBM_BO_USE_PROTECTED) != 0 {
        return 0;
    }
    if format_to_bpp(format).is_some() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_device_get_format_modifier_plane_count(
    gbm: *mut gbm_device,
    format: u32,
    modifier: u64,
) -> c_int {
    if gbm.is_null() {
        return -1;
    }
    if (modifier == DRM_FORMAT_MOD_LINEAR || modifier == DRM_FORMAT_MOD_INVALID)
        && format_to_bpp(format).is_some()
    {
        1
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_device_destroy(gbm: *mut gbm_device) {
    if !gbm.is_null() {
        drop(Box::from_raw(gbm));
    }
}

// ============================================================================
// C-ABI: Quản lý Buffer Object (gbm_bo)
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_create(
    gbm: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    flags: u32,
) -> *mut gbm_bo {
    if gbm.is_null() || width == 0 || height == 0 {
        return ptr::null_mut();
    }
    let norm_fmt = normalize_format(format);
    let bpp = match format_to_bpp(norm_fmt) {
        Some(b) => b,
        None => return ptr::null_mut(),
    };

    let dev = &*gbm;
    let alloc = match create_dumb_buffer(dev.drm_fd, width, height, bpp, dev.allow_memfd_fallback) {
        Ok(a) => a,
        Err(_) => return ptr::null_mut(),
    };

    let bo = Box::new(gbm_bo {
        device: gbm,
        width,
        height,
        format: norm_fmt,
        bpp,
        flags,
        modifier: DRM_FORMAT_MOD_LINEAR,
        offset: 0,
        alloc,
        imported: false,
        cached_dmabuf_fd: Mutex::new(None),
        user_data: Mutex::new(UserDataEntry {
            data: ptr::null_mut(),
            destroy_fn: None,
        }),
    });

    Box::into_raw(bo)
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_create_with_modifiers(
    gbm: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    modifiers: *const u64,
    count: u32,
) -> *mut gbm_bo {
    gbm_bo_create_with_modifiers2(
        gbm,
        width,
        height,
        format,
        modifiers,
        count,
        GBM_BO_USE_RENDERING | GBM_BO_USE_SCANOUT,
    )
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_create_with_modifiers2(
    gbm: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    modifiers: *const u64,
    count: u32,
    flags: u32,
) -> *mut gbm_bo {
    let modifier = match select_modifier(modifiers, count) {
        Some(m) => m,
        None => return ptr::null_mut(),
    };
    let bo = gbm_bo_create(gbm, width, height, format, flags | GBM_BO_USE_LINEAR);
    if !bo.is_null() {
        (*bo).modifier = modifier;
    }
    bo
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_import(
    gbm: *mut gbm_device,
    import_type: u32,
    buffer: *mut c_void,
    usage: u32,
) -> *mut gbm_bo {
    if gbm.is_null() || buffer.is_null() {
        return ptr::null_mut();
    }
    let dev = &*gbm;

    match import_type {
        GBM_BO_IMPORT_FD => {
            let data = &*(buffer as *const gbm_import_fd_data);
            if data.fd < 0 || data.width == 0 || data.height == 0 || data.stride == 0 {
                return ptr::null_mut();
            }
            let norm_fmt = normalize_format(data.format);
            let bpp = match format_to_bpp(norm_fmt) {
                Some(b) => b,
                None => return ptr::null_mut(),
            };

            let _ = FenceSyncManager::wait_dmabuf_ready(data.fd, DEFAULT_FENCE_TIMEOUT_MS);

            let (handle, is_mock, backing_fd) =
                match import_prime_fd(dev.drm_fd, data.fd, dev.allow_memfd_fallback) {
                    Ok(res) => res,
                    Err(_) => return ptr::null_mut(),
                };

            let size = (data.stride as u64).saturating_mul(data.height as u64);
            let alloc = DumbBufferAllocation {
                handle,
                pitch: data.stride,
                size,
                is_mock_memfd: is_mock,
                backing_fd,
            };

            Box::into_raw(Box::new(gbm_bo {
                device: gbm,
                width: data.width,
                height: data.height,
                format: norm_fmt,
                bpp,
                flags: usage,
                modifier: DRM_FORMAT_MOD_LINEAR,
                offset: 0,
                alloc,
                imported: true,
                cached_dmabuf_fd: Mutex::new(None),
                user_data: Mutex::new(UserDataEntry {
                    data: ptr::null_mut(),
                    destroy_fn: None,
                }),
            }))
        }
        GBM_BO_IMPORT_FD_MODIFIER => {
            let data = &*(buffer as *const gbm_import_fd_modifier_data);
            if data.num_fds == 0
                || data.fds[0] < 0
                || data.width == 0
                || data.height == 0
                || data.strides[0] <= 0
                || data.offsets[0] < 0
            {
                return ptr::null_mut();
            }
            let norm_fmt = normalize_format(data.format);
            let bpp = match format_to_bpp(norm_fmt) {
                Some(b) => b,
                None => return ptr::null_mut(),
            };

            let _ = FenceSyncManager::wait_dmabuf_ready(data.fds[0], DEFAULT_FENCE_TIMEOUT_MS);

            let (handle, is_mock, backing_fd) =
                match import_prime_fd(dev.drm_fd, data.fds[0], dev.allow_memfd_fallback) {
                    Ok(res) => res,
                    Err(_) => return ptr::null_mut(),
                };

            let stride = data.strides[0] as u32;
            let offset = data.offsets[0] as u32;
            let size = (stride as u64)
                .saturating_mul(data.height as u64)
                .saturating_add(offset as u64);

            let alloc = DumbBufferAllocation {
                handle,
                pitch: stride,
                size,
                is_mock_memfd: is_mock,
                backing_fd,
            };

            Box::into_raw(Box::new(gbm_bo {
                device: gbm,
                width: data.width,
                height: data.height,
                format: norm_fmt,
                bpp,
                flags: usage,
                modifier: data.modifier,
                offset,
                alloc,
                imported: true,
                cached_dmabuf_fd: Mutex::new(None),
                user_data: Mutex::new(UserDataEntry {
                    data: ptr::null_mut(),
                    destroy_fn: None,
                }),
            }))
        }
        _ => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_map(
    bo: *mut gbm_bo,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    flags: u32,
    stride: *mut u32,
    map_data: *mut *mut c_void,
) -> *mut c_void {
    if bo.is_null() || stride.is_null() || map_data.is_null() || width == 0 || height == 0 {
        return ptr::null_mut();
    }
    let bo_ref = &*bo;
    if x.saturating_add(width) > bo_ref.width || y.saturating_add(height) > bo_ref.height {
        return ptr::null_mut();
    }

    let dev = &*bo_ref.device;
    let write_access = (flags & GBM_BO_TRANSFER_WRITE) != 0;
    let mut prot = libc::PROT_READ;
    if write_access {
        prot |= libc::PROT_WRITE;
    }

    let dmabuf_fd = gbm_bo_get_fd(bo);
    if dmabuf_fd >= 0 {
        let _ = FenceSyncManager::wait_dmabuf_ready(dmabuf_fd, DEFAULT_FENCE_TIMEOUT_MS);
        let _ = FenceSyncManager::begin_cpu_access(dmabuf_fd, write_access);
    }

    let base_ptr = match map_dumb_buffer(dev.drm_fd, &bo_ref.alloc, prot) {
        Ok(p) => p,
        Err(_) => {
            if dmabuf_fd >= 0 {
                let _ = FenceSyncManager::end_cpu_access(dmabuf_fd, write_access);
                libc::close(dmabuf_fd);
            }
            return ptr::null_mut();
        }
    };

    let bytes_per_pixel = bo_ref.bpp.div_ceil(8);
    let byte_offset = (bo_ref.offset as usize)
        .saturating_add((y as usize).saturating_mul(bo_ref.alloc.pitch as usize))
        .saturating_add((x as usize).saturating_mul(bytes_per_pixel as usize));

    *stride = bo_ref.alloc.pitch;
    let region = Box::new(MappedRegion {
        base_ptr,
        length: bo_ref.alloc.size as usize,
        dmabuf_fd,
        write_access,
    });
    *map_data = Box::into_raw(region).cast::<c_void>();

    base_ptr.cast::<u8>().add(byte_offset).cast::<c_void>()
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_unmap(bo: *mut gbm_bo, map_data: *mut c_void) {
    if bo.is_null() || map_data.is_null() {
        return;
    }
    let region = Box::from_raw(map_data.cast::<MappedRegion>());
    libc::munmap(region.base_ptr, region.length as size_t);
    if region.dmabuf_fd >= 0 {
        let _ = FenceSyncManager::end_cpu_access(region.dmabuf_fd, region.write_access);
        libc::close(region.dmabuf_fd);
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_width(bo: *mut gbm_bo) -> u32 {
    if bo.is_null() {
        0
    } else {
        (*bo).width
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_height(bo: *mut gbm_bo) -> u32 {
    if bo.is_null() {
        0
    } else {
        (*bo).height
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_stride(bo: *mut gbm_bo) -> u32 {
    if bo.is_null() {
        0
    } else {
        (*bo).alloc.pitch
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_stride_for_plane(bo: *mut gbm_bo, plane: c_int) -> u32 {
    if bo.is_null() || plane != 0 {
        0
    } else {
        (*bo).alloc.pitch
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_format(bo: *mut gbm_bo) -> u32 {
    if bo.is_null() {
        0
    } else {
        (*bo).format
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_bpp(bo: *mut gbm_bo) -> u32 {
    if bo.is_null() {
        0
    } else {
        (*bo).bpp
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_offset(bo: *mut gbm_bo, plane: c_int) -> u32 {
    if bo.is_null() || plane != 0 {
        0
    } else {
        (*bo).offset
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_device(bo: *mut gbm_bo) -> *mut gbm_device {
    if bo.is_null() {
        ptr::null_mut()
    } else {
        (*bo).device
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_handle(bo: *mut gbm_bo) -> gbm_bo_handle {
    if bo.is_null() {
        gbm_bo_handle { u64_: 0 }
    } else {
        gbm_bo_handle {
            u32_: (*bo).alloc.handle,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_handle_for_plane(
    bo: *mut gbm_bo,
    plane: c_int,
) -> gbm_bo_handle {
    if bo.is_null() || plane != 0 {
        gbm_bo_handle { s32: -1 }
    } else {
        gbm_bo_handle {
            u32_: (*bo).alloc.handle,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_fd(bo: *mut gbm_bo) -> c_int {
    if bo.is_null() {
        return -1;
    }
    let bo_ref = &*bo;
    let dev = &*bo_ref.device;

    let fd = match export_prime_fd(dev.drm_fd, &bo_ref.alloc) {
        Ok(fd) => fd,
        Err(_) => return -1,
    };

    let _ = FenceSyncManager::wait_dmabuf_ready(fd, DEFAULT_FENCE_TIMEOUT_MS);

    if let Ok(mut cached) = bo_ref.cached_dmabuf_fd.lock() {
        if cached.is_none() {
            let dup_fd = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0);
            if dup_fd >= 0 {
                *cached = Some(dup_fd);
            }
        }
    }

    fd
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_fd_for_plane(bo: *mut gbm_bo, plane: c_int) -> c_int {
    if plane != 0 {
        return -1;
    }
    gbm_bo_get_fd(bo)
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_modifier(bo: *mut gbm_bo) -> u64 {
    if bo.is_null() {
        DRM_FORMAT_MOD_INVALID
    } else {
        (*bo).modifier
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_plane_count(bo: *mut gbm_bo) -> c_int {
    if bo.is_null() {
        0
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_set_user_data(
    bo: *mut gbm_bo,
    data: *mut c_void,
    destroy_user_data: GbmDestroyUserDataFn,
) {
    if bo.is_null() {
        return;
    }
    if let Ok(mut guard) = (*bo).user_data.lock() {
        *guard = UserDataEntry {
            data,
            destroy_fn: destroy_user_data,
        };
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_get_user_data(bo: *mut gbm_bo) -> *mut c_void {
    if bo.is_null() {
        return ptr::null_mut();
    }
    match (*bo).user_data.lock() {
        Ok(guard) => guard.data,
        Err(_) => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_bo_destroy(bo: *mut gbm_bo) {
    if bo.is_null() {
        return;
    }

    let user_entry = (*bo).user_data.lock().ok().map(|mut g| {
        let entry = UserDataEntry {
            data: g.data,
            destroy_fn: g.destroy_fn,
        };
        g.data = ptr::null_mut();
        g.destroy_fn = None;
        entry
    });

    if let Some(entry) = user_entry {
        if let Some(cb) = entry.destroy_fn {
            cb(bo, entry.data);
        }
    }

    let mut boxed_bo = Box::from_raw(bo);
    if let Ok(cached) = boxed_bo.cached_dmabuf_fd.get_mut() {
        if let Some(fd) = cached.take() {
            libc::close(fd);
        }
    }

    if !boxed_bo.device.is_null() {
        let drm_fd = (*boxed_bo.device).drm_fd;
        destroy_dumb_buffer(drm_fd, &boxed_bo.alloc, boxed_bo.imported);
    }
}

// ============================================================================
// C-ABI: Quản lý Surface (Triple-Buffered Swapchain cho Wayland Compositor)
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_create(
    gbm: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    flags: u32,
) -> *mut gbm_surface {
    if gbm.is_null() || width == 0 || height == 0 {
        return ptr::null_mut();
    }
    let norm_fmt = normalize_format(format);
    if format_to_bpp(norm_fmt).is_none() {
        return ptr::null_mut();
    }

    let mut slots: Vec<SurfaceSlot> = Vec::with_capacity(SURFACE_BUFFER_COUNT);
    for _ in 0..SURFACE_BUFFER_COUNT {
        let bo = gbm_bo_create(gbm, width, height, norm_fmt, flags);
        if bo.is_null() {
            for existing in slots {
                gbm_bo_destroy(existing.bo);
            }
            return ptr::null_mut();
        }
        slots.push(SurfaceSlot {
            bo,
            state: SlotState::Free,
        });
    }

    Box::into_raw(Box::new(gbm_surface {
        device: gbm,
        width,
        height,
        format: norm_fmt,
        flags,
        modifier: DRM_FORMAT_MOD_LINEAR,
        slots: Mutex::new(slots),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_create_with_modifiers(
    gbm: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    modifiers: *const u64,
    count: u32,
) -> *mut gbm_surface {
    gbm_surface_create_with_modifiers2(
        gbm,
        width,
        height,
        format,
        modifiers,
        count,
        GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING,
    )
}

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_create_with_modifiers2(
    gbm: *mut gbm_device,
    width: u32,
    height: u32,
    format: u32,
    modifiers: *const u64,
    count: u32,
    flags: u32,
) -> *mut gbm_surface {
    let modifier = match select_modifier(modifiers, count) {
        Some(m) => m,
        None => return ptr::null_mut(),
    };
    let surface = gbm_surface_create(gbm, width, height, format, flags | GBM_BO_USE_LINEAR);
    if !surface.is_null() {
        (*surface).modifier = modifier;
    }
    surface
}

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_lock_front_buffer(surface: *mut gbm_surface) -> *mut gbm_bo {
    if surface.is_null() {
        return ptr::null_mut();
    }
    let surf = &*surface;
    let mut slots = match surf.slots.lock() {
        Ok(g) => g,
        Err(_) => return ptr::null_mut(),
    };

    for slot in slots.iter_mut() {
        if slot.state == SlotState::Free {
            if let Ok(cached_fd_guard) = (*slot.bo).cached_dmabuf_fd.lock() {
                if let Some(fd) = *cached_fd_guard {
                    let _ = FenceSyncManager::wait_dmabuf_ready(fd, DEFAULT_FENCE_TIMEOUT_MS);
                }
            }
            slot.state = SlotState::Locked;
            return slot.bo;
        }
    }

    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_release_buffer(surface: *mut gbm_surface, bo: *mut gbm_bo) {
    if surface.is_null() || bo.is_null() {
        return;
    }
    let surf = &*surface;
    if let Ok(mut slots) = surf.slots.lock() {
        for slot in slots.iter_mut() {
            if slot.bo == bo {
                slot.state = SlotState::Free;
                break;
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_has_free_buffers(surface: *mut gbm_surface) -> c_int {
    if surface.is_null() {
        return 0;
    }
    let surf = &*surface;
    match surf.slots.lock() {
        Ok(slots) => {
            if slots.iter().any(|s| s.state == SlotState::Free) {
                1
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn gbm_surface_destroy(surface: *mut gbm_surface) {
    if surface.is_null() {
        return;
    }
    let mut boxed_surface = Box::from_raw(surface);
    if let Ok(slots) = boxed_surface.slots.get_mut() {
        for slot in slots.drain(..) {
            if !slot.bo.is_null() {
                gbm_bo_destroy(slot.bo);
            }
        }
    }
    drop(boxed_surface);
}

// ============================================================================
// Automated Unit Tests (Chạy độc lập với `cargo test`)
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::egl_shim::{
        eglDestroySurface, translate_create_window_surface, translate_gbm_to_device_display,
        translate_swap_buffers, EGLBoolean, EGLConfig, EGLDeviceEXT, EGLDisplay, EGLSurface,
        EGLenum, EGLint, EGL_NO_DISPLAY, EGL_PLATFORM_DEVICE_EXT, EGL_PLATFORM_GBM_KHR, EGL_TRUE,
    };
    use libc::c_uint;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn create_mock_drm_fd() -> c_int {
        unsafe { libc::memfd_create(c"mock_drm_node".as_ptr(), libc::MFD_CLOEXEC) }
    }

    #[test]
    fn test_device_and_bo_lifecycle() {
        let drm_fd = create_mock_drm_fd();
        assert!(drm_fd >= 0);

        unsafe {
            let dev = gbm_create_device(drm_fd);
            assert!(!dev.is_null());
            assert_eq!(gbm_device_get_fd(dev), drm_fd);
            assert_eq!(
                gbm_device_is_format_supported(dev, GBM_FORMAT_XRGB8888, GBM_BO_USE_SCANOUT),
                1
            );
            assert_eq!(
                gbm_device_is_format_supported(dev, GBM_FORMAT_XRGB8888, GBM_BO_USE_PROTECTED),
                0
            );

            let bo = gbm_bo_create(
                dev,
                1920,
                1080,
                GBM_FORMAT_ARGB8888,
                GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING,
            );
            assert!(!bo.is_null());
            assert_eq!(gbm_bo_get_width(bo), 1920);
            assert_eq!(gbm_bo_get_height(bo), 1080);
            assert_eq!(gbm_bo_get_format(bo), GBM_FORMAT_ARGB8888);
            assert!(gbm_bo_get_stride(bo) >= 1920 * 4);
            assert_eq!(gbm_bo_get_plane_count(bo), 1);
            assert_eq!(gbm_bo_get_modifier(bo), DRM_FORMAT_MOD_LINEAR);

            let dmabuf_fd = gbm_bo_get_fd(bo);
            assert!(dmabuf_fd >= 0);

            let mut stride: u32 = 0;
            let mut map_data: *mut c_void = ptr::null_mut();
            let mapped_ptr = gbm_bo_map(
                bo,
                0,
                0,
                1920,
                1080,
                GBM_BO_TRANSFER_READ_WRITE,
                &mut stride,
                &mut map_data,
            );
            assert!(!mapped_ptr.is_null());
            assert!(!map_data.is_null());

            let pixel_ptr = mapped_ptr.cast::<u32>();
            *pixel_ptr = 0xFF11_2233;
            assert_eq!(*pixel_ptr, 0xFF11_2233);

            gbm_bo_unmap(bo, map_data);
            libc::close(dmabuf_fd);
            gbm_bo_destroy(bo);
            gbm_device_destroy(dev);
            libc::close(drm_fd);
        }
    }

    static DESTRUCTOR_CALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn on_bo_destroy(_bo: *mut gbm_bo, data: *mut c_void) {
        let flag = &*(data as *const AtomicBool);
        flag.store(true, Ordering::SeqCst);
    }

    #[test]
    fn test_user_data_destructor_and_surface_swapchain() {
        let drm_fd = create_mock_drm_fd();
        unsafe {
            let dev = gbm_create_device(drm_fd);
            let surface = gbm_surface_create(
                dev,
                1280,
                720,
                GBM_FORMAT_XRGB8888,
                GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING,
            );
            assert!(!surface.is_null());
            assert_eq!(gbm_surface_has_free_buffers(surface), 1);

            let bo1 = gbm_surface_lock_front_buffer(surface);
            let bo2 = gbm_surface_lock_front_buffer(surface);
            let bo3 = gbm_surface_lock_front_buffer(surface);
            assert!(!bo1.is_null());
            assert!(!bo2.is_null());
            assert!(!bo3.is_null());

            assert_eq!(gbm_surface_has_free_buffers(surface), 0);
            assert!(gbm_surface_lock_front_buffer(surface).is_null());

            DESTRUCTOR_CALLED.store(false, Ordering::SeqCst);
            gbm_bo_set_user_data(
                bo1,
                (&DESTRUCTOR_CALLED as *const AtomicBool)
                    .cast_mut()
                    .cast::<c_void>(),
                Some(on_bo_destroy),
            );

            gbm_surface_release_buffer(surface, bo1);
            assert_eq!(gbm_surface_has_free_buffers(surface), 1);

            gbm_surface_destroy(surface);
            assert!(DESTRUCTOR_CALLED.load(Ordering::SeqCst));

            gbm_device_destroy(dev);
            libc::close(drm_fd);
        }
    }

    #[test]
    fn test_fence_sync_polling_with_eventfd() {
        unsafe {
            let efd = libc::eventfd(1, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK);
            assert!(efd >= 0);
            let res = FenceSyncManager::poll_sync_fd(efd, 100);
            assert!(res.is_ok());
            libc::close(efd);
        }
    }

    unsafe extern "C" fn mock_nvidia_470_get_platform_display_ext(
        platform: EGLenum,
        native_display: *mut c_void,
        _attrib_list: *const EGLint,
    ) -> EGLDisplay {
        if platform == EGL_PLATFORM_GBM_KHR {
            return EGL_NO_DISPLAY;
        }
        if platform == EGL_PLATFORM_DEVICE_EXT && !native_display.is_null() {
            return 0xDEAD_BEEFusize as EGLDisplay;
        }
        EGL_NO_DISPLAY
    }

    unsafe extern "C" fn mock_nvidia_470_query_devices_ext(
        max_devices: EGLint,
        devices: *mut EGLDeviceEXT,
        num_devices: *mut EGLint,
    ) -> EGLBoolean {
        if max_devices > 0 && !devices.is_null() && !num_devices.is_null() {
            *devices = 0xCAFE_BABEusize as EGLDeviceEXT;
            *num_devices = 1;
            return EGL_TRUE;
        }
        0
    }

    unsafe extern "C" fn mock_create_pbuffer(
        _dpy: EGLDisplay,
        _config: EGLConfig,
        _attrib_list: *const EGLint,
    ) -> EGLSurface {
        0x7777_8888usize as EGLSurface
    }

    unsafe extern "C" fn mock_gl_read_pixels(
        _x: i32,
        _y: i32,
        width: i32,
        height: i32,
        _format: c_uint,
        _gl_type: c_uint,
        pixels: *mut c_void,
    ) {
        let count = (width as usize).saturating_mul(height as usize);
        let slice = std::slice::from_raw_parts_mut(pixels.cast::<u32>(), count);
        for px in slice.iter_mut() {
            *px = 0xFFAA_BBCC;
        }
    }

    #[test]
    fn test_egl_gbm_to_device_ext_and_swapbuffers_blit_pipeline() {
        let drm_fd = create_mock_drm_fd();
        unsafe {
            let dev = gbm_create_device(drm_fd);
            let dpy = translate_gbm_to_device_display(
                EGL_PLATFORM_GBM_KHR,
                dev.cast::<c_void>(),
                ptr::null(),
                mock_nvidia_470_get_platform_display_ext,
                Some(mock_nvidia_470_query_devices_ext),
            );
            assert_eq!(dpy, 0xDEAD_BEEFusize as EGLDisplay);

            let surf = gbm_surface_create(
                dev,
                64,
                64,
                GBM_FORMAT_XRGB8888,
                GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING,
            );
            assert!(!surf.is_null());

            let egl_surf = translate_create_window_surface(
                dpy,
                ptr::null_mut(),
                surf.cast::<c_void>(),
                ptr::null(),
                None,
                mock_create_pbuffer,
            );
            assert_eq!(egl_surf, 0x7777_8888usize as EGLSurface);

            let swapped = translate_swap_buffers(dpy, egl_surf, None, Some(mock_gl_read_pixels));
            assert_eq!(swapped, EGL_TRUE);

            let front_bo = gbm_surface_lock_front_buffer(surf);
            assert!(!front_bo.is_null());

            let mut stride: u32 = 0;
            let mut map_data: *mut c_void = ptr::null_mut();
            let ptr = gbm_bo_map(
                front_bo,
                0,
                0,
                64,
                64,
                GBM_BO_TRANSFER_READ,
                &mut stride,
                &mut map_data,
            );
            assert!(!ptr.is_null());
            assert_eq!(*ptr.cast::<u32>(), 0xFFAA_BBCC);
            gbm_bo_unmap(front_bo, map_data);

            gbm_surface_release_buffer(surf, front_bo);
            assert_eq!(eglDestroySurface(dpy, egl_surf), EGL_TRUE);
            gbm_surface_destroy(surf);
            gbm_device_destroy(dev);
            libc::close(drm_fd);
        }
    }
}
