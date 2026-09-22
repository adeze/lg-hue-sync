use anyhow::{anyhow, Result};
use libloading::{Library, Symbol};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use tracing::{error, info, warn};

use super::{CapturedFrame, ScreenCapture};

pub const LIBVTCAPTURE_PATH: &str = "/usr/lib/libvtcapture.so.1";

/// Properties passed to `vtCapture_preprocess` (20 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibVtCaptureProperties {
    /// 0 = SCALER_INPUT (HDMI video stream before scaler)
    /// 1 = SCALER_OUTPUT (after hardware scaler)
    /// 2 = DISPLAY_OUTPUT (composited display output)
    pub dump: c_int,
    pub loc_x: i16,
    pub loc_y: i16,
    pub reg_w: i16,
    pub reg_h: i16,
    pub buf_cnt: c_int,
    pub frm: c_int,
}

/// Plane layout information returned by `vtCapture_planeInfo` (20 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LibVtCapturePlaneInfo {
    pub stride: c_int,
    pub plane_x: i16,
    pub plane_y: i16,
    pub plane_w: i16,
    pub plane_h: i16,
    pub active_x: i16,
    pub active_y: i16,
    pub active_w: i16,
    pub active_h: i16,
}

/// Active frame buffer memory addresses from `vtCapture_currentCaptureBuffInfo` (16 bytes on 32-bit ARM).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LibVtCaptureBufferInfo {
    /// Pointer to Y plane (luma)
    pub start_addr0: *mut u8,
    /// Pointer to UV interleaved plane (chroma NV12)
    pub start_addr1: *mut u8,
    pub size0: c_int,
    pub size1: c_int,
}

type FnVtCaptureCreate = unsafe extern "C" fn() -> *mut c_void;
type FnVtCaptureInit = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char) -> c_int;
type FnVtCapturePreprocess =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const LibVtCaptureProperties) -> c_int;
type FnVtCapturePlaneInfo =
    unsafe extern "C" fn(*mut c_void, *const c_char, *mut LibVtCapturePlaneInfo) -> c_int;
type FnVtCaptureProcess = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FnVtCaptureCurrentCaptureBuffInfo =
    unsafe extern "C" fn(*mut c_void, *mut LibVtCaptureBufferInfo) -> c_int;
type FnVtCaptureStop = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FnVtCapturePostprocess = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FnVtCaptureFinalize = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type FnVtCaptureRelease = unsafe extern "C" fn(*mut c_void) -> c_int;

/// Native libvtcapture driver implementation for LG webOS 6.x (LG C1 OLED).
pub struct VtCapture {
    driver: *mut c_void,
    client_id: [u8; 128],
    width: u32,
    height: u32,
    stride: u32,
    rgb_buffer: Vec<u8>,
    is_stopped: bool,
    consecutive_errors: u32,
    last_addr0: usize,
    stale_count: u32,
    fn_current_buff_info: FnVtCaptureCurrentCaptureBuffInfo,
    fn_stop: FnVtCaptureStop,
    fn_postprocess: FnVtCapturePostprocess,
    fn_finalize: FnVtCaptureFinalize,
    fn_release: FnVtCaptureRelease,
    _lib: Library,
}

// Raw pointers in VtCapture are actor/thread safe when owned by ScreenCapture
unsafe impl Send for VtCapture {}

impl VtCapture {
    /// Attempts to dynamically load `/usr/lib/libvtcapture.so.1` and initialize the capture pipeline.
    pub fn try_new(target_width: u32, target_height: u32) -> Result<Self> {
        if !Path::new(LIBVTCAPTURE_PATH).exists() {
            return Err(anyhow!(
                "libvtcapture driver library not found at {}",
                LIBVTCAPTURE_PATH
            ));
        }

        info!(
            "Loading webOS libvtcapture driver from {}...",
            LIBVTCAPTURE_PATH
        );
        let lib = unsafe { Library::new(LIBVTCAPTURE_PATH) }
            .map_err(|e| anyhow!("Failed to dlopen {}: {}", LIBVTCAPTURE_PATH, e))?;

        unsafe {
            let fn_create: Symbol<FnVtCaptureCreate> = lib
                .get(b"vtCapture_create\0")
                .map_err(|e| anyhow!("Symbol vtCapture_create not found: {}", e))?;
            let fn_init: Symbol<FnVtCaptureInit> = lib
                .get(b"vtCapture_init\0")
                .map_err(|e| anyhow!("Symbol vtCapture_init not found: {}", e))?;
            let fn_preprocess: Symbol<FnVtCapturePreprocess> =
                lib.get(b"vtCapture_preprocess\0")
                    .map_err(|e| anyhow!("Symbol vtCapture_preprocess not found: {}", e))?;
            let fn_plane_info: Symbol<FnVtCapturePlaneInfo> = lib
                .get(b"vtCapture_planeInfo\0")
                .map_err(|e| anyhow!("Symbol vtCapture_planeInfo not found: {}", e))?;
            let fn_process: Symbol<FnVtCaptureProcess> = lib
                .get(b"vtCapture_process\0")
                .map_err(|e| anyhow!("Symbol vtCapture_process not found: {}", e))?;
            let fn_current_buff_info: Symbol<FnVtCaptureCurrentCaptureBuffInfo> = lib
                .get(b"vtCapture_currentCaptureBuffInfo\0")
                .map_err(|e| anyhow!("Symbol vtCapture_currentCaptureBuffInfo not found: {}", e))?;
            let fn_stop: Symbol<FnVtCaptureStop> = lib
                .get(b"vtCapture_stop\0")
                .map_err(|e| anyhow!("Symbol vtCapture_stop not found: {}", e))?;
            let fn_postprocess: Symbol<FnVtCapturePostprocess> = lib
                .get(b"vtCapture_postprocess\0")
                .map_err(|e| anyhow!("Symbol vtCapture_postprocess not found: {}", e))?;
            let fn_finalize: Symbol<FnVtCaptureFinalize> = lib
                .get(b"vtCapture_finalize\0")
                .map_err(|e| anyhow!("Symbol vtCapture_finalize not found: {}", e))?;
            let fn_release: Symbol<FnVtCaptureRelease> = lib
                .get(b"vtCapture_release\0")
                .map_err(|e| anyhow!("Symbol vtCapture_release not found: {}", e))?;

            // 1. Create driver instance
            let driver = fn_create();
            if driver.is_null() {
                return Err(anyhow!("vtCapture_create returned NULL"));
            }

            // 2. Initialize with caller ID and initial "00" client ID buffer
            let mut client_id = [0u8; 128];
            client_id[0] = b'0';
            client_id[1] = b'0';
            let caller_id = CString::new("org.webosbrew.lg-hue-sync")?;

            let ret_init = fn_init(
                driver,
                caller_id.as_ptr(),
                client_id.as_mut_ptr() as *mut c_char,
            );
            if ret_init != 0 {
                fn_release(driver);
                if ret_init == 11 {
                    error!(
                        "[-] vtCapture_init returned error 11 (EAGAIN/EBUSY): /dev/video* hardware scaler is busy! Another capture process (e.g. lg-hue-sync.service, PicCap) is already running. Stop it with: systemctl stop lg-hue-sync"
                    );
                    return Err(anyhow!(
                        "vtCapture_init failed with error code 11 (EBUSY / /dev/video* device is busy)"
                    ));
                }
                return Err(anyhow!(
                    "vtCapture_init failed with error code {}",
                    ret_init
                ));
            }

            let client_str = CStr::from_ptr(client_id.as_ptr() as *const c_char).to_string_lossy();
            info!(
                "[+] vtCapture_init successful (caller: {}, client_id: '{}')",
                caller_id.to_string_lossy(),
                client_str
            );

            // 3. Preprocess properties (try DISPLAY_OUTPUT=2 first, then SCALER_INPUT=0, then SCALER_OUTPUT=1)
            // DISPLAY_OUTPUT taps the composited display output and does not lock the raw HDMI input clock during mode switches.
            let mut preprocessed = false;
            let mut last_code = -1;
            let client_ptr = client_id.as_ptr() as *const c_char;

            for dump_mode in [2, 0, 1] {
                let props = LibVtCaptureProperties {
                    dump: dump_mode,
                    loc_x: 0,
                    loc_y: 0,
                    reg_w: target_width as i16,
                    reg_h: target_height as i16,
                    buf_cnt: 3,
                    frm: 60,
                };
                let ret_pre = fn_preprocess(driver, client_ptr, &props);
                if ret_pre == 0 {
                    info!(
                        "[+] vtCapture_preprocess succeeded (dump_mode: {}, resolution: {}x{})",
                        dump_mode, target_width, target_height
                    );
                    preprocessed = true;
                    break;
                }
                last_code = ret_pre;
            }

            if !preprocessed {
                fn_finalize(driver, client_ptr);
                fn_release(driver);
                return Err(anyhow!(
                    "vtCapture_preprocess failed across all dump modes (last code: {})",
                    last_code
                ));
            }

            // 4. Query plane layout and hardware stride
            let mut plane_info = LibVtCapturePlaneInfo::default();
            let ret_plane = fn_plane_info(driver, client_ptr, &mut plane_info);
            if ret_plane != 0 {
                warn!(
                    "vtCapture_planeInfo returned code {}, using target width as stride",
                    ret_plane
                );
            }

            // Honor actual hardware-configured plane dimensions if reported, otherwise fallback to target
            let width = if plane_info.plane_w > 0 {
                plane_info.plane_w as u32
            } else {
                target_width
            };
            let height = if plane_info.plane_h > 0 {
                plane_info.plane_h as u32
            } else {
                target_height
            };

            let stride = if plane_info.stride > 0 && plane_info.stride as u32 >= width {
                plane_info.stride as u32
            } else {
                width
            };

            info!(
                "[+] vtCapture plane layout: stride={}, plane={}x{} (target: {}x{}), active={}x{}",
                stride,
                width,
                height,
                target_width,
                target_height,
                plane_info.active_w,
                plane_info.active_h
            );

            // 5. Start streaming capture process
            let ret_proc = fn_process(driver, client_ptr);
            if ret_proc != 0 {
                fn_stop(driver, client_ptr);
                fn_postprocess(driver, client_ptr);
                fn_finalize(driver, client_ptr);
                fn_release(driver);
                return Err(anyhow!("vtCapture_process failed with code {}", ret_proc));
            }

            info!("[+] vtCapture_process stream started successfully");

            let rgb_size = (width * height * 4) as usize;

            Ok(Self {
                driver,
                client_id,
                width,
                height,
                stride,
                rgb_buffer: vec![0; rgb_size],
                is_stopped: false,
                consecutive_errors: 0,
                last_addr0: 0,
                stale_count: 0,
                fn_current_buff_info: *fn_current_buff_info,
                fn_stop: *fn_stop,
                fn_postprocess: *fn_postprocess,
                fn_finalize: *fn_finalize,
                fn_release: *fn_release,
                _lib: lib,
            })
        }
    }

    /// Explicitly stops and finalizes capture to release the hardware scaler in the webOS kernel.
    pub fn stop_stream(&mut self) {
        if self.is_stopped || self.driver.is_null() {
            return;
        }
        unsafe {
            let client_ptr = self.client_id.as_ptr() as *const c_char;
            (self.fn_stop)(self.driver, client_ptr);
            (self.fn_postprocess)(self.driver, client_ptr);
            (self.fn_finalize)(self.driver, client_ptr);
            self.is_stopped = true;
        }
        info!("[-] Released vtCapture hardware scaler channel.");
    }

    /// Converts NV12 (Y plane + interleaved UV plane) to standard 32-bit RGBA using fixed-point BT.601 limited-to-full range.
    #[inline(always)]
    unsafe fn convert_nv12_to_rgb(&mut self, y_plane: *const u8, uv_plane: *const u8) {
        convert_nv12_to_rgba(
            self.width,
            self.height,
            self.stride,
            y_plane,
            uv_plane,
            &mut self.rgb_buffer,
        );
    }
}

/// Converts NV12 (Y plane + interleaved UV plane) to standard 32-bit RGBA using fixed-point BT.601 limited-to-full range.
/// Optimizes ARM execution by pairing adjacent horizontal pixels to reuse chroma (U, V) calculations.
#[inline(always)]
pub fn convert_nv12_to_rgba(
    width: u32,
    height: u32,
    stride: u32,
    y_plane: *const u8,
    uv_plane: *const u8,
    rgb_buffer: &mut [u8],
) {
    let w = width as usize;
    let h = height as usize;
    let s = stride as usize;

    for y in 0..h {
        let y_row = y * s;
        let uv_row = (y / 2) * s;
        let out_row = y * w * 4;

        for x in (0..w).step_by(2) {
            let uv_offset = uv_row + x;
            let u_val = unsafe { *uv_plane.add(uv_offset) as i32 - 128 };
            let v_val = unsafe { *uv_plane.add(uv_offset + 1) as i32 - 128 };

            // Shared chroma components for both horizontal pixels
            let rv = 409 * v_val + 128;
            let gv = -100 * u_val - 208 * v_val + 128;
            let bv = 516 * u_val + 128;

            // First pixel (x)
            let y0 = unsafe { (*y_plane.add(y_row + x) as i32 - 16).max(0) };
            let c0 = 298 * y0;
            let r0 = ((c0 + rv) >> 8).clamp(0, 255) as u8;
            let g0 = ((c0 + gv) >> 8).clamp(0, 255) as u8;
            let b0 = ((c0 + bv) >> 8).clamp(0, 255) as u8;

            let out0 = out_row + x * 4;
            rgb_buffer[out0] = r0;
            rgb_buffer[out0 + 1] = g0;
            rgb_buffer[out0 + 2] = b0;
            rgb_buffer[out0 + 3] = 255;

            // Second pixel (x + 1) if within bounds
            if x + 1 < w {
                let y1 = unsafe { (*y_plane.add(y_row + x + 1) as i32 - 16).max(0) };
                let c1 = 298 * y1;
                let r1 = ((c1 + rv) >> 8).clamp(0, 255) as u8;
                let g1 = ((c1 + gv) >> 8).clamp(0, 255) as u8;
                let b1 = ((c1 + bv) >> 8).clamp(0, 255) as u8;

                let out1 = out0 + 4;
                rgb_buffer[out1] = r1;
                rgb_buffer[out1 + 1] = g1;
                rgb_buffer[out1 + 2] = b1;
                rgb_buffer[out1 + 3] = 255;
            }
        }
    }
}

impl ScreenCapture for VtCapture {
    fn acquire_frame(&mut self) -> Result<CapturedFrame<'_>> {
        if self.is_stopped {
            return Err(anyhow!("VIDEO_FORMAT_SWITCH"));
        }

        let mut buf_info = LibVtCaptureBufferInfo {
            start_addr0: std::ptr::null_mut(),
            start_addr1: std::ptr::null_mut(),
            size0: 0,
            size1: 0,
        };

        let ret = unsafe { (self.fn_current_buff_info)(self.driver, &mut buf_info) };
        if ret != 0 || buf_info.start_addr0.is_null() {
            self.consecutive_errors += 1;
            if self.consecutive_errors >= 4 {
                warn!(
                    "Hardware capture error (ret={}, null buffer) for {} frames: stopping pipeline to release scaler...",
                    ret, self.consecutive_errors
                );
                self.stop_stream();
                return Err(anyhow!("VIDEO_FORMAT_SWITCH"));
            }
            // Transient frame drop: reuse previous frame
            return Ok(CapturedFrame {
                data: &self.rgb_buffer,
                width: self.width,
                height: self.height,
                is_bgra: false,
            });
        }

        self.consecutive_errors = 0;

        let curr_addr = buf_info.start_addr0 as usize;
        if curr_addr == self.last_addr0 {
            self.stale_count += 1;
            // Unchanged address for > 1.5s (90 frames at 60 FPS) indicates frozen video pipe / timing change
            if self.stale_count >= 90 {
                warn!("Hardware capture buffer address unchanged for 1.5s: resetting pipeline to prevent display mute...");
                self.stop_stream();
                return Err(anyhow!("VIDEO_FORMAT_SWITCH"));
            }
        } else {
            self.last_addr0 = curr_addr;
            self.stale_count = 0;
        }

        unsafe {
            if !buf_info.start_addr1.is_null() {
                self.convert_nv12_to_rgb(buf_info.start_addr0, buf_info.start_addr1);
            } else {
                // Grayscale fallback if only Y plane is provided
                let width = self.width as usize;
                let height = self.height as usize;
                let stride = self.stride as usize;
                for y in 0..height {
                    for x in 0..width {
                        let y_val = *buf_info.start_addr0.add(y * stride + x);
                        let out_idx = (y * width + x) * 4;
                        self.rgb_buffer[out_idx] = y_val;
                        self.rgb_buffer[out_idx + 1] = y_val;
                        self.rgb_buffer[out_idx + 2] = y_val;
                        self.rgb_buffer[out_idx + 3] = 255;
                    }
                }
            }
        }

        Ok(CapturedFrame {
            data: &self.rgb_buffer,
            width: self.width,
            height: self.height,
            is_bgra: false,
        })
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_real_hardware(&self) -> bool {
        true
    }
}

impl Drop for VtCapture {
    fn drop(&mut self) {
        unsafe {
            if !self.driver.is_null() {
                let client_ptr = self.client_id.as_ptr() as *const c_char;
                if !self.is_stopped {
                    (self.fn_stop)(self.driver, client_ptr);
                    (self.fn_postprocess)(self.driver, client_ptr);
                    (self.fn_finalize)(self.driver, client_ptr);
                }
                (self.fn_release)(self.driver);
                self.driver = std::ptr::null_mut();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vtcapture_struct_sizes() {
        assert_eq!(std::mem::size_of::<LibVtCaptureProperties>(), 20);
        assert_eq!(std::mem::size_of::<LibVtCapturePlaneInfo>(), 20);
        #[cfg(target_pointer_width = "32")]
        assert_eq!(std::mem::size_of::<LibVtCaptureBufferInfo>(), 16);
    }

    #[test]
    fn test_vtcapture_fails_gracefully_when_library_missing() {
        let res = VtCapture::try_new(160, 90);
        assert!(res.is_err());
    }

    #[test]
    fn test_nv12_conversion_black_and_white_reference() {
        // 2x2 test image
        let width = 2u32;
        let height = 2u32;
        let stride = 2u32;

        // Test 1: Standard video black (Y=16, U=128, V=128)
        let y_black = [16u8; 4];
        let uv_neutral = [128u8; 2]; // 1 pair for 2x2 NV12
        let mut rgb_out = [0u8; 16];

        convert_nv12_to_rgba(
            width,
            height,
            stride,
            y_black.as_ptr(),
            uv_neutral.as_ptr(),
            &mut rgb_out,
        );

        // Every pixel should decode to black (0, 0, 0, 255)
        for pixel in rgb_out.chunks(4) {
            assert_eq!(pixel, &[0, 0, 0, 255]);
        }

        // Test 2: Standard video white (Y=235, U=128, V=128)
        let y_white = [235u8; 4];
        convert_nv12_to_rgba(
            width,
            height,
            stride,
            y_white.as_ptr(),
            uv_neutral.as_ptr(),
            &mut rgb_out,
        );

        // Every pixel should decode to full white (255, 255, 255, 255)
        for pixel in rgb_out.chunks(4) {
            assert_eq!(pixel, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn test_nv12_conversion_hardware_stride_handling() {
        // Test that a plane with stride larger than width correctly skips padding bytes
        // 2x2 image embedded inside stride=4 buffer
        let width = 2u32;
        let height = 2u32;
        let stride = 4u32;

        // Row 0: [235, 235, padding, padding]
        // Row 1: [16, 16, padding, padding]
        let y_data = [235u8, 235u8, 0, 0, 16u8, 16u8, 0, 0];
        // UV row 0 (applies to both Y rows): [128, 128, padding, padding]
        let uv_data = [128u8, 128u8, 0, 0];
        let mut rgb_out = [0u8; 16];

        convert_nv12_to_rgba(
            width,
            height,
            stride,
            y_data.as_ptr(),
            uv_data.as_ptr(),
            &mut rgb_out,
        );

        // Row 0 pixels must be white
        assert_eq!(&rgb_out[0..4], &[255, 255, 255, 255]);
        assert_eq!(&rgb_out[4..8], &[255, 255, 255, 255]);

        // Row 1 pixels must be black (verifying stride=4 advanced correctly to index 4)
        assert_eq!(&rgb_out[8..12], &[0, 0, 0, 255]);
        assert_eq!(&rgb_out[12..16], &[0, 0, 0, 255]);
    }

    #[test]
    fn test_nv12_conversion_odd_width_boundary() {
        // 3x2 image to test boundary condition where width is not a multiple of 2
        let width = 3u32;
        let height = 2u32;
        let stride = 4u32;

        let y_data = [235u8, 235u8, 235u8, 0, 16u8, 16u8, 16u8, 0];
        let uv_data = [128u8, 128u8, 128u8, 128u8];
        let mut rgb_out = [0u8; 24]; // 3 * 2 * 4

        convert_nv12_to_rgba(
            width,
            height,
            stride,
            y_data.as_ptr(),
            uv_data.as_ptr(),
            &mut rgb_out,
        );

        // Row 0: 3 white pixels
        assert_eq!(&rgb_out[0..4], &[255, 255, 255, 255]);
        assert_eq!(&rgb_out[4..8], &[255, 255, 255, 255]);
        assert_eq!(&rgb_out[8..12], &[255, 255, 255, 255]);

        // Row 1: 3 black pixels
        assert_eq!(&rgb_out[12..16], &[0, 0, 0, 255]);
        assert_eq!(&rgb_out[16..20], &[0, 0, 0, 255]);
        assert_eq!(&rgb_out[20..24], &[0, 0, 0, 255]);
    }
}
