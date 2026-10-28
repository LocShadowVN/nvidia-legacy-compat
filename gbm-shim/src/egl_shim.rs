use crate::{gbm_surface, gbm_surface_blit_pixels};
use libc::{c_char, c_uint, c_void};
use std::ptr;
use std::sync::Mutex;

pub type EGLenum = c_uint;
pub type EGLint = i32;
pub type EGLAttrib = isize;
pub type EGLBoolean = c_uint;
pub type EGLDisplay = *mut c_void;
pub type EGLConfig = *mut c_void;
pub type EGLSurface = *mut c_void;
pub type EGLDeviceEXT = *mut c_void;

pub const EGL_FALSE: EGLBoolean = 0;
pub const EGL_TRUE: EGLBoolean = 1;
pub const EGL_NONE: EGLint = 0x3038;
pub const EGL_WIDTH: EGLint = 0x3057;
pub const EGL_HEIGHT: EGLint = 0x3056;
pub const EGL_NO_DISPLAY: EGLDisplay = ptr::null_mut();
pub const EGL_NO_SURFACE: EGLSurface = ptr::null_mut();

// Chuẩn Khronos EGL Platform Constants
pub const EGL_PLATFORM_GBM_KHR: EGLenum = 0x31D7;
pub const EGL_PLATFORM_GBM_MESA: EGLenum = 0x31D7;
pub const EGL_PLATFORM_DEVICE_EXT: EGLenum = 0x313F;

// Chuẩn OpenGL Pixel Readback Constants
pub const GL_BGRA_EXT: c_uint = 0x80E1;
pub const GL_UNSIGNED_BYTE: c_uint = 0x1401;

pub type EglGetProcAddressFn = unsafe extern "C" fn(procname: *const c_char) -> *mut c_void;
pub type EglGetPlatformDisplayFn = unsafe extern "C" fn(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLAttrib,
) -> EGLDisplay;
pub type EglGetPlatformDisplayExtFn = unsafe extern "C" fn(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLint,
) -> EGLDisplay;
pub type EglQueryDevicesExtFn = unsafe extern "C" fn(
    max_devices: EGLint,
    devices: *mut EGLDeviceEXT,
    num_devices: *mut EGLint,
) -> EGLBoolean;
pub type EglCreatePlatformWindowSurfaceFn = unsafe extern "C" fn(
    dpy: EGLDisplay,
    config: EGLConfig,
    native_window: *mut c_void,
    attrib_list: *const EGLAttrib,
) -> EGLSurface;
pub type EglCreateWindowSurfaceFn = unsafe extern "C" fn(
    dpy: EGLDisplay,
    config: EGLConfig,
    win: *mut c_void,
    attrib_list: *const EGLint,
) -> EGLSurface;
pub type EglCreatePbufferSurfaceFn = unsafe extern "C" fn(
    dpy: EGLDisplay,
    config: EGLConfig,
    attrib_list: *const EGLint,
) -> EGLSurface;
pub type EglSwapBuffersFn =
    unsafe extern "C" fn(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
pub type EglDestroySurfaceFn =
    unsafe extern "C" fn(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
pub type GlReadPixelsFn = unsafe extern "C" fn(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    format: c_uint,
    gl_type: c_uint,
    pixels: *mut c_void,
);

#[derive(Clone, Copy)]
struct SurfaceBinding {
    dpy: usize,
    egl_surface: usize,
    gbm_surf: *mut gbm_surface,
}

unsafe impl Send for SurfaceBinding {}

static SURFACE_BINDINGS: Mutex<Vec<SurfaceBinding>> = Mutex::new(Vec::new());

fn register_surface_binding(dpy: EGLDisplay, egl_surface: EGLSurface, gbm_surf: *mut gbm_surface) {
    if let Ok(mut list) = SURFACE_BINDINGS.lock() {
        list.retain(|b| !(b.dpy == dpy as usize && b.egl_surface == egl_surface as usize));
        list.push(SurfaceBinding {
            dpy: dpy as usize,
            egl_surface: egl_surface as usize,
            gbm_surf,
        });
    }
}

fn lookup_surface_binding(dpy: EGLDisplay, egl_surface: EGLSurface) -> Option<*mut gbm_surface> {
    if let Ok(list) = SURFACE_BINDINGS.lock() {
        for b in list.iter() {
            if b.dpy == dpy as usize && b.egl_surface == egl_surface as usize {
                return Some(b.gbm_surf);
            }
        }
    }
    None
}

fn remove_surface_binding(dpy: EGLDisplay, egl_surface: EGLSurface) {
    if let Ok(mut list) = SURFACE_BINDINGS.lock() {
        list.retain(|b| !(b.dpy == dpy as usize && b.egl_surface == egl_surface as usize));
    }
}

/// Tìm symbol gốc trong chuỗi thư viện động (libEGL_nvidia.so.0 / libEGL.so.1 / libGLESv2.so.2)
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

/// Chuyển đổi EGL_PLATFORM_GBM_KHR sang EGL_PLATFORM_DEVICE_EXT cho driver 470.xx/390.xx
pub unsafe fn translate_gbm_to_device_display(
    platform: EGLenum,
    native_display: *mut c_void,
    attrib_list: *const EGLint,
    real_get_platform_display_ext: EglGetPlatformDisplayExtFn,
    real_query_devices_ext: Option<EglQueryDevicesExtFn>,
) -> EGLDisplay {
    let direct_dpy = real_get_platform_display_ext(platform, native_display, attrib_list);
    if !direct_dpy.is_null() {
        return direct_dpy;
    }

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

/// Tạo Pbuffer Surface phần cứng trên GPU NVIDIA gắn kết với `gbm_surface`
/// khi `libEGL_nvidia.so.0` (470.xx/390.xx) từ chối tạo Window Surface từ `gbm_surface`.
pub unsafe fn translate_create_window_surface(
    dpy: EGLDisplay,
    config: EGLConfig,
    native_window: *mut c_void,
    attrib_list: *const EGLint,
    real_create_window: Option<EglCreateWindowSurfaceFn>,
    real_create_pbuffer: EglCreatePbufferSurfaceFn,
) -> EGLSurface {
    if dpy.is_null() || native_window.is_null() {
        return EGL_NO_SURFACE;
    }

    if let Some(win_fn) = real_create_window {
        let direct_surf = win_fn(dpy, config, native_window, attrib_list);
        if !direct_surf.is_null() {
            return direct_surf;
        }
    }

    let gbm_surf = native_window.cast::<gbm_surface>();
    let width = (*gbm_surf).width as EGLint;
    let height = (*gbm_surf).height as EGLint;
    if width <= 0 || height <= 0 {
        return EGL_NO_SURFACE;
    }

    let pbuffer_attribs: [EGLint; 5] = [EGL_WIDTH, width, EGL_HEIGHT, height, EGL_NONE];
    let pbuffer = real_create_pbuffer(dpy, config, pbuffer_attribs.as_ptr());
    if !pbuffer.is_null() {
        register_surface_binding(dpy, pbuffer, gbm_surf);
    }
    pbuffer
}

/// Khi Compositor gọi `eglSwapBuffers`, nếu surface là Pbuffer gắn với `gbm_surface`,
/// tự động chép frame vừa render từ GPU sang `gbm_bo` (DMA-BUF) để DRM/KMS quét ra màn hình.
pub unsafe fn translate_swap_buffers(
    dpy: EGLDisplay,
    surface: EGLSurface,
    real_swap: Option<EglSwapBuffersFn>,
    read_pixels_fn: Option<GlReadPixelsFn>,
) -> EGLBoolean {
    if dpy.is_null() || surface.is_null() {
        return EGL_FALSE;
    }

    if let Some(gbm_surf) = lookup_surface_binding(dpy, surface) {
        let resolved_read_pixels = read_pixels_fn.or_else(|| {
            let ptr = resolve_next_symbol(c"glReadPixels");
            if ptr.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*mut c_void, GlReadPixelsFn>(ptr))
            }
        });

        if let Some(gl_read) = resolved_read_pixels {
            let _ = gbm_surface_blit_pixels(gbm_surf, gl_read);
        }

        if let Some(swap_fn) = real_swap {
            let _ = swap_fn(dpy, surface);
        }
        return EGL_TRUE;
    }

    if let Some(swap_fn) = real_swap {
        return swap_fn(dpy, surface);
    }
    EGL_FALSE
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

    if platform == EGL_PLATFORM_GBM_KHR {
        let empty_attribs: [EGLint; 1] = [EGL_NONE];
        return eglGetPlatformDisplayEXT(platform, native_display, empty_attribs.as_ptr());
    }

    EGL_NO_DISPLAY
}

#[no_mangle]
pub unsafe extern "C" fn eglCreateWindowSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    win: *mut c_void,
    attrib_list: *const EGLint,
) -> EGLSurface {
    let win_ptr = resolve_next_symbol(c"eglCreateWindowSurface");
    let win_fn = if win_ptr.is_null() {
        None
    } else {
        Some(std::mem::transmute::<*mut c_void, EglCreateWindowSurfaceFn>(win_ptr))
    };

    let pbuf_ptr = resolve_next_symbol(c"eglCreatePbufferSurface");
    if pbuf_ptr.is_null() {
        return win_fn
            .map(|f| f(dpy, config, win, attrib_list))
            .unwrap_or(EGL_NO_SURFACE);
    }
    let pbuf_fn: EglCreatePbufferSurfaceFn = std::mem::transmute(pbuf_ptr);

    translate_create_window_surface(dpy, config, win, attrib_list, win_fn, pbuf_fn)
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePlatformWindowSurfaceEXT(
    dpy: EGLDisplay,
    config: EGLConfig,
    native_window: *mut c_void,
    attrib_list: *const EGLint,
) -> EGLSurface {
    eglCreateWindowSurface(dpy, config, native_window, attrib_list)
}

#[no_mangle]
pub unsafe extern "C" fn eglCreatePlatformWindowSurface(
    dpy: EGLDisplay,
    config: EGLConfig,
    native_window: *mut c_void,
    attrib_list: *const EGLAttrib,
) -> EGLSurface {
    let real_ptr = resolve_next_symbol(c"eglCreatePlatformWindowSurface");
    if !real_ptr.is_null() {
        let real_fn: EglCreatePlatformWindowSurfaceFn = std::mem::transmute(real_ptr);
        let surf = real_fn(dpy, config, native_window, attrib_list);
        if !surf.is_null() {
            return surf;
        }
    }

    let empty_attribs: [EGLint; 1] = [EGL_NONE];
    eglCreateWindowSurface(dpy, config, native_window, empty_attribs.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean {
    let swap_ptr = resolve_next_symbol(c"eglSwapBuffers");
    let swap_fn = if swap_ptr.is_null() {
        None
    } else {
        Some(std::mem::transmute::<*mut c_void, EglSwapBuffersFn>(
            swap_ptr,
        ))
    };
    translate_swap_buffers(dpy, surface, swap_fn, None)
}

#[no_mangle]
pub unsafe extern "C" fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean {
    remove_surface_binding(dpy, surface);
    let destroy_ptr = resolve_next_symbol(c"eglDestroySurface");
    if !destroy_ptr.is_null() {
        let destroy_fn: EglDestroySurfaceFn = std::mem::transmute(destroy_ptr);
        return destroy_fn(dpy, surface);
    }
    EGL_TRUE
}
