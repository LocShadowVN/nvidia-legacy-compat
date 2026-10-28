use libc::{c_char, c_int, c_uint, c_void};
use std::ptr;

pub type EGLenum = c_uint;
pub type EGLint = i32;
pub type EGLAttrib = isize;
pub type EGLBoolean = c_uint;
pub type EGLDisplay = *mut c_void;
pub type EGLDeviceEXT = *mut c_void;

pub const EGL_FALSE: EGLBoolean = 0;
pub const EGL_TRUE: EGLBoolean = 1;
pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_NO_DISPLAY: EGLDisplay = ptr::null_mut();

// Chuẩn Khronos EGL Platform Constants
pub const EGL_PLATFORM_GBM_KHR: EGLenum = 0x31D7;
pub const EGL_PLATFORM_GBM_MESA: EGLenum = 0x31D7;
pub const EGL_PLATFORM_DEVICE_EXT: EGLenum = 0x313F;

type EglGetProcAddressFn = unsafe extern "C" fn(procname: *const c_char) -> *mut c_void;
type EglGetPlatformDisplayFn = unsafe extern "C" fn(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLAttrib,
) -> EGLDisplay;
type EglGetPlatformDisplayExtFn = unsafe extern "C" fn(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLint,
) -> EGLDisplay;
type EglQueryDevicesExtFn = unsafe extern "C" fn(
    max_devices: EGLint,
    devices: *mut EGLDeviceEXT,
    num_devices: *mut EGLint,
) -> EGLBoolean;

/// Tìm symbol gốc trong chuỗi thư viện động (libEGL_nvidia.so.0 / libEGL.so.1)
unsafe fn resolve_next_symbol(name: &std::ffi::CStr) -> *mut c_void {
    let sym = libc::dlsym(libc::RTLD_NEXT, name.as_ptr());
    if !sym.is_null() {
        return sym;
    }

    let get_proc_ptr = libc::dlsym(libc::RTLD_NEXT, c"eglGetProcAddress".as_ptr());
    if !get_proc_ptr.is_null() {
        let get_proc: EglGetProcAddressFn = std::mem::transmute(get_proc_ptr);
        return get_proc(name.as_ptr());
    }
    ptr::null_mut()
}

/// Dò tìm EGLDeviceEXT đầu tiên của GPU NVIDIA thông qua eglQueryDevicesEXT
pub unsafe fn query_first_nvidia_egl_device(
    query_fn: Option<EglQueryDevicesExtFn>,
) -> Option<EGLDeviceEXT> {
    let func = match query_fn {
        Some(f) => f,
        None => {
            let ptr = resolve_next_symbol(c"eglQueryDevicesEXT");
            if ptr.is_null() {
                return None;
            }
            std::mem::transmute::<*mut c_void, EglQueryDevicesExtFn>(ptr)
        }
    };

    let mut devices: [EGLDeviceEXT; 8] = [ptr::null_mut(); 8];
    let mut num_devices: EGLint = 0;
    let ok = func(8, devices.as_mut_ptr(), &mut num_devices);
    if ok == EGL_TRUE && num_devices > 0 && !devices[0].is_null() {
        Some(devices[0])
    } else {
        None
    }
}

/// Logic chuyển đổi cốt lõi: Thử EGL_PLATFORM_GBM_KHR trước; nếu driver 470.xx/390.xx
/// trả về EGL_NO_DISPLAY thì tự động fallback sang EGL_PLATFORM_DEVICE_EXT.
pub unsafe fn translate_gbm_to_device_display(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLint,
    real_get_platform_display_ext: EglGetPlatformDisplayExtFn,
    real_query_devices_ext: Option<EglQueryDevicesExtFn>,
) -> EGLDisplay {
    // 1. Gọi thử với platform gốc (nếu chạy trên Mesa hoặc driver >= 495 sẽ thành công ngay)
    let direct_dpy = real_get_platform_display_ext(platform, native_display, attrib_list);
    if !direct_dpy.is_null() {
        return direct_dpy;
    }

    // 2. Nếu platform là GBM và bị libEGL_nvidia.so.0 (470.xx/390.xx) từ chối,
    // chuyển hướng sang EGL_PLATFORM_DEVICE_EXT
    if platform == EGL_PLATFORM_GBM_KHR {
        if let Some(egl_dev) = query_first_nvidia_egl_device(real_query_devices_ext) {
            let empty_attribs: [EGLint; 1] = [EGL_NONE];
            let fallback_dpy = real_get_platform_display_ext(
                EGL_PLATFORM_DEVICE_EXT,
                egl_dev,
                empty_attribs.as_ptr(),
            );
            if !fallback_dpy.is_null() {
                return fallback_dpy;
            }
        }
    }

    EGL_NO_DISPLAY
}

#[no_mangle]
pub unsafe extern "C" fn eglGetPlatformDisplayEXT(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLint,
) -> EGLDisplay {
    let real_ptr = resolve_next_symbol(c"eglGetPlatformDisplayEXT");
    if real_ptr.is_null() {
        return EGL_NO_DISPLAY;
    }
    let real_fn: EglGetPlatformDisplayExtFn = std::mem::transmute(real_ptr);
    translate_gbm_to_device_display(platform, native_display, attrib_list, real_fn, None)
}

#[no_mangle]
pub unsafe extern "C" fn eglGetPlatformDisplay(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLAttrib,
) -> EGLDisplay {
    let real_ptr = resolve_next_symbol(c"eglGetPlatformDisplay");
    if !real_ptr.is_null() {
        let real_fn: EglGetPlatformDisplayFn = std::mem::transmute(real_ptr);
        let dpy = real_fn(platform, native_display, attrib_list);
        if !dpy.is_null() {
            return dpy;
        }
    }

    // Fallback qua eglGetPlatformDisplayEXT với EGL_PLATFORM_DEVICE_EXT
    if platform == EGL_PLATFORM_GBM_KHR {
        let empty_attribs: [EGLint; 1] = [EGL_NONE];
        return eglGetPlatformDisplayEXT(platform, native_display, empty_attribs.as_ptr());
    }

    EGL_NO_DISPLAY
}

#[allow(dead_code)]
pub const fn is_gbm_platform(platform: c_int) -> bool {
    (platform as u32) == EGL_PLATFORM_GBM_KHR
}
