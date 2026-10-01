use anyhow::{anyhow, Result};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::time::Instant;
use tracing::{info, warn};

pub use super::{CaptureError, CapturedFrame, NativeCaptureError, ScreenCapture};

// -----------------------------------------------------------------------------
// Mock Capture (for macOS host, local tests, and CI)
// -----------------------------------------------------------------------------

pub struct MockCapture {
    width: u32,
    height: u32,
    buffer: Vec<u8>,
    start_time: Instant,
    is_tv_hardware: bool,
}

impl MockCapture {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height * 4) as usize;
        let is_tv_hardware = std::path::Path::new("/usr/lib/libvtcapture.so.1").exists()
            || std::path::Path::new("/usr/lib/libdile_vt.so.0").exists();
        Self {
            width,
            height,
            buffer: vec![0; size],
            start_time: Instant::now(),
            is_tv_hardware,
        }
    }
}

impl ScreenCapture for MockCapture {
    fn acquire_frame(&mut self) -> std::result::Result<CapturedFrame<'_>, CaptureError> {
        if self.is_tv_hardware {
            // On real TV hardware, keep buffer black during fallback/settling
            // rather than distracting rainbow test pattern
            return Ok(CapturedFrame {
                data: &self.buffer,
                width: self.width,
                height: self.height,
                is_bgra: false,
            });
        }

        let elapsed = self.start_time.elapsed().as_secs_f32();
        let base_hue = (elapsed * 45.0) % 360.0;

        for y in 0..self.height {
            let row_offset = (y * self.width * 4) as usize;
            for x in 0..self.width {
                let pixel_offset = row_offset + (x * 4) as usize;
                let hue = (base_hue + (x as f32 / self.width as f32) * 180.0) % 360.0;
                let (r, g, b) = hsv_to_rgb(hue, 0.9, 0.9);
                self.buffer[pixel_offset] = r;
                self.buffer[pixel_offset + 1] = g;
                self.buffer[pixel_offset + 2] = b;
                self.buffer[pixel_offset + 3] = 255;
            }
        }

        Ok(CapturedFrame {
            data: &self.buffer,
            width: self.width,
            height: self.height,
            is_bgra: false,
        })
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

// -----------------------------------------------------------------------------
// DILE_VT Native Driver (Direct In-display Video Texture for LG webOS 6.x)
// -----------------------------------------------------------------------------

#[allow(dead_code)]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DileVtPixelFormat {
    Yuv420Planar = 0,
    Yuv420SemiPlanar = 1, // NV12: Plane 0 = Y, Plane 1 = interleaved UV
    Yuv420Interleaved = 2,
    Yuv422Planar = 3,
    Yuv422SemiPlanar = 4,
    Yuv422Interleaved = 5,
    Rgb = 9,
    Argb = 10,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DileOutputDeviceState {
    pub enabled: u8,
    pub freezed: u8,
    pub applied_pq: u8,
    pub unknown: u8,
    pub framerate: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DileVtRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DileVtFramebufferCapability {
    pub num_vfbs: u32,
    pub num_planes: u32,
}

#[repr(C)]
#[derive(Debug)]
pub struct DileVtFramebufferProperty {
    pub pixel_format: u32,
    pub stride: u32,
    pub width: u32,
    pub height: u32,
    pub ptr: *mut *mut u32, // [num_vfbs][num_planes] of physical DMA offsets
}

const STATE_FREEZED: u32 = 0x02;
const STATE_FRAMERATE_DIVIDE: u32 = 0x10;
const DUMP_SCALER_OUTPUT: i32 = 0;
const DUMP_DISPLAY_OUTPUT: i32 = 1;

struct MappedPlane {
    data_ptr: *mut u8,
    mapped_base: *mut c_void,
    map_len: usize,
    data_len: usize,
}

impl Drop for MappedPlane {
    fn drop(&mut self) {
        // SAFETY: this mapping was returned by mmap and remains owned by this plane.
        unsafe { libc::munmap(self.mapped_base, self.map_len) };
    }
}

type FnCreate = unsafe extern "C" fn(u32) -> *mut c_void;
type FnCreateEx = unsafe extern "C" fn(u32, i32) -> *mut c_void;
type FnStart = unsafe extern "C" fn(*mut c_void) -> i32;
type FnStop = unsafe extern "C" fn(*mut c_void) -> i32;
type FnDestroy = unsafe extern "C" fn(*mut c_void);
type FnSetDumpLocation = unsafe extern "C" fn(*mut c_void, i32) -> i32;
type FnSetOutputRegion = unsafe extern "C" fn(*mut c_void, i32, *const DileVtRect) -> i32;
type FnSetState = unsafe extern "C" fn(*mut c_void, u32, *const DileOutputDeviceState) -> i32;
type FnGetCapability = unsafe extern "C" fn(*mut c_void, *mut DileVtFramebufferCapability) -> i32;
type FnGetAllProperties = unsafe extern "C" fn(
    *mut c_void,
    *const DileVtFramebufferCapability,
    *mut DileVtFramebufferProperty,
) -> i32;
type FnGetCurrentProperty = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut u32) -> i32;
type FnWaitVsync = unsafe extern "C" fn(*mut c_void) -> i32;

struct SetupHandle {
    handle: *mut c_void,
    destroy: FnDestroy,
}

impl Drop for SetupHandle {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: this handle is owned by setup until transferred to DileVtCapture.
            unsafe { (self.destroy)(self.handle) };
        }
    }
}

pub struct DileVtCapture {
    handle: *mut c_void,
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: u32,
    num_vfbs: usize,
    #[allow(dead_code)]
    num_planes: usize,
    // [vfb_idx][plane_idx] -> MappedPlane
    mapped_buffers: Vec<Vec<MappedPlane>>,
    rgb_buffer: Vec<u8>,
    _mem_file: File,
    // Dynamic symbols
    fn_stop: FnStop,
    fn_destroy: FnDestroy,
    fn_get_current: FnGetCurrentProperty,
    fn_wait_vsync: FnWaitVsync,
    // Hold libraries in memory
    _lib_pmlog: Option<libloading::os::unix::Library>,
    _lib_hal: Option<libloading::os::unix::Library>,
    _lib_dile: libloading::Library,
}

impl DileVtCapture {
    pub fn try_new(target_width: u32, target_height: u32, fps_limit: u32) -> Result<Self> {
        if target_width == 0
            || target_height == 0
            || target_width > u16::MAX as u32
            || target_height > u16::MAX as u32
        {
            return Err(anyhow!("capture dimensions exceed DILE_VT limits"));
        }
        let dile_path = "/usr/lib/libdile_vt.so.0";
        if !std::path::Path::new(dile_path).exists() {
            return Err(anyhow!("Driver {} not found on system", dile_path));
        }

        info!(
            "Found webOS display driver at {}. Loading dependencies...",
            dile_path
        );

        use libloading::os::unix::{Library as UnixLib, RTLD_GLOBAL, RTLD_NOW};

        // Preload supporting libraries with RTLD_GLOBAL so _PmLogMsgKV and HAL symbols resolve globally
        let lib_pmlog = unsafe {
            UnixLib::open(Some("/usr/lib/libPmLogLib.so.3"), RTLD_NOW | RTLD_GLOBAL).ok()
        };
        let lib_hal =
            unsafe { UnixLib::open(Some("/usr/lib/libhal_vt.so.2"), RTLD_NOW | RTLD_GLOBAL).ok() };

        let unix_lib = unsafe { UnixLib::open(Some(dile_path), RTLD_NOW | RTLD_GLOBAL) }
            .map_err(|e| anyhow!("Failed to load {}: {}", dile_path, e))?;
        let lib = libloading::Library::from(unix_lib);

        unsafe {
            let fn_create_ex: Option<libloading::Symbol<FnCreateEx>> =
                lib.get(b"DILE_VT_CreateEx\0").ok();
            let fn_create: libloading::Symbol<FnCreate> = lib.get(b"DILE_VT_Create\0")?;
            let fn_start: libloading::Symbol<FnStart> = lib.get(b"DILE_VT_Start\0")?;
            let fn_stop: libloading::Symbol<FnStop> = lib.get(b"DILE_VT_Stop\0")?;
            let fn_destroy: libloading::Symbol<FnDestroy> = lib.get(b"DILE_VT_Destroy\0")?;
            let fn_set_dump: libloading::Symbol<FnSetDumpLocation> =
                lib.get(b"DILE_VT_SetVideoFrameOutputDeviceDumpLocation\0")?;
            let fn_set_region: libloading::Symbol<FnSetOutputRegion> =
                lib.get(b"DILE_VT_SetVideoFrameOutputDeviceOutputRegion\0")?;
            let fn_set_state: libloading::Symbol<FnSetState> =
                lib.get(b"DILE_VT_SetVideoFrameOutputDeviceState\0")?;
            let fn_get_cap: libloading::Symbol<FnGetCapability> =
                lib.get(b"DILE_VT_GetVideoFrameBufferCapability\0")?;
            let fn_get_all: libloading::Symbol<FnGetAllProperties> =
                lib.get(b"DILE_VT_GetAllVideoFrameBufferProperty\0")?;
            let fn_get_current: libloading::Symbol<FnGetCurrentProperty> =
                lib.get(b"DILE_VT_GetCurrentVideoFrameBufferProperty\0")?;
            let fn_wait_vsync: libloading::Symbol<FnWaitVsync> = lib.get(b"DILE_VT_WaitVsync\0")?;

            // 1. Initialize context:
            // On webOS 6.x (LG C1 Alpha 9 Gen 4), DILE_VT_CreateEx(0, 1) MUST be called first (QUIRK_DILE_VT_CREATE_EX).
            // Calling DILE_VT_Create(0) first attempts to allocate 5 buffers which fails and leaks /dev/video60,
            // causing subsequent CreateEx calls to fail with -EBUSY.
            // Never call DILE_VT_Init() as it opens /dev/video60 directly.
            let mut handle = std::ptr::null_mut();

            if let Some(ref create_ex) = fn_create_ex {
                info!("Attempting DILE_VT_CreateEx(0, 1) (QUIRK_DILE_VT_CREATE_EX)...");
                handle = create_ex(0, 1);
                if handle.is_null() {
                    info!("DILE_VT_CreateEx(0, 1) returned NULL, trying DILE_VT_CreateEx(0, 2)...");
                    handle = create_ex(0, 2);
                }
            }

            if handle.is_null() {
                info!("CreateEx not available or returned NULL, attempting DILE_VT_Create(0)...");
                handle = fn_create(0);
            }

            if handle.is_null() {
                return Err(anyhow!(
                    "Failed to acquire DILE_VT context (both CreateEx and Create returned NULL)"
                ));
            }
            let mut setup = SetupHandle {
                handle,
                destroy: *fn_destroy,
            };

            info!("[+] Successfully acquired DILE_VT handle: {:?}", handle);

            // 2. Configure dump location: DISPLAY_OUTPUT with SCALER_OUTPUT fallback
            let mut dump_location = DUMP_DISPLAY_OUTPUT;
            if fn_set_dump(handle, dump_location) != 0 {
                warn!("DISPLAY_OUTPUT rejected, falling back to SCALER_OUTPUT");
                dump_location = DUMP_SCALER_OUTPUT;
                if fn_set_dump(handle, dump_location) != 0 {
                    return Err(anyhow!("Failed to set DILE_VT dump location"));
                }
            }

            // 3. Configure hardware downscaling region directly in silicon
            let region = DileVtRect {
                x: 0,
                y: 0,
                width: target_width as u16,
                height: target_height as u16,
            };
            if fn_set_region(handle, dump_location, &region) != 0 {
                return Err(anyhow!(
                    "Failed to set DILE_VT output region to {}x{}",
                    target_width,
                    target_height
                ));
            }

            // 4. Configure framerate divider and ensure unfreezed state
            let divider = 60u32.checked_div(fps_limit).unwrap_or(1).max(1);
            let state = DileOutputDeviceState {
                enabled: 0,
                freezed: 0,
                applied_pq: 0,
                unknown: 0,
                framerate: divider,
            };
            let _ = fn_set_state(handle, STATE_FRAMERATE_DIVIDE, &state);
            let _ = fn_set_state(handle, STATE_FREEZED, &state);

            // 5. Query buffer capability (ring buffers and planes)
            let mut cap = DileVtFramebufferCapability {
                num_vfbs: 0,
                num_planes: 0,
            };
            if fn_get_cap(handle, &mut cap) != 0 || cap.num_vfbs == 0 || cap.num_planes == 0 {
                return Err(anyhow!("Failed to get DILE_VT capability"));
            }

            // 6. Allocate pointer matrix and query physical memory addresses
            let mut ptr_rows: Vec<Vec<u32>> =
                vec![vec![0; cap.num_planes as usize]; cap.num_vfbs as usize];
            let mut ptr_ptrs: Vec<*mut u32> = ptr_rows.iter_mut().map(|r| r.as_mut_ptr()).collect();

            let mut prop = DileVtFramebufferProperty {
                pixel_format: DileVtPixelFormat::Yuv420SemiPlanar as u32,
                stride: 0,
                width: 0,
                height: 0,
                ptr: ptr_ptrs.as_mut_ptr(),
            };

            if fn_get_all(handle, &cap, &mut prop) != 0 {
                return Err(anyhow!("DILE_VT_GetAllVideoFrameBufferProperty failed"));
            }

            info!(
                "[+] DILE_VT hardware video plane ready: {}x{}, stride: {}, format: {:?}, ring buffers: {}, planes: {}",
                prop.width, prop.height, prop.stride, prop.pixel_format, cap.num_vfbs, cap.num_planes
            );

            // 7. Open /dev/mem with O_SYNC to map physical DMA video buffers
            let mem_file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_SYNC)
                .open("/dev/mem")
                .map_err(|e| anyhow!("Failed to open /dev/mem (requires root): {}", e))?;

            let mem_fd = mem_file.as_raw_fd();
            let page_size = libc::sysconf(libc::_SC_PAGESIZE);
            if page_size <= 0 {
                return Err(anyhow!("could not determine system page size"));
            }
            let page_size = page_size as usize;
            let page_mask = (page_size - 1) as libc::off_t;

            let capture_width = if prop.width > 0 {
                prop.width
            } else {
                target_width
            };
            let capture_height = if prop.height > 0 {
                prop.height
            } else {
                target_height
            };
            let stride = if prop.stride > 0 {
                prop.stride
            } else {
                capture_width
            };
            let (_, _, rgb_size) =
                super::vtcapture::nv12_lengths(capture_width, capture_height, stride)?;
            if prop.pixel_format == DileVtPixelFormat::Yuv420SemiPlanar as u32 && cap.num_planes < 2
            {
                return Err(anyhow!("NV12 capture requires two planes"));
            }
            if prop.pixel_format == DileVtPixelFormat::Rgb as u32
                && (stride as usize) < rgb_size / capture_height as usize
            {
                return Err(anyhow!("RGB capture stride is shorter than a pixel row"));
            }

            let mut mapped_buffers = Vec::with_capacity(cap.num_vfbs as usize);

            for (vfb_idx, ptr_row) in ptr_rows.iter().take(cap.num_vfbs as usize).enumerate() {
                let mut planes = Vec::with_capacity(cap.num_planes as usize);
                for (plane_idx, &ptr) in ptr_row.iter().take(cap.num_planes as usize).enumerate() {
                    let phys_addr = ptr as libc::off_t;
                    let phys_base = phys_addr & !page_mask;
                    let page_offset = (phys_addr & page_mask) as usize;

                    let rows = if plane_idx == 0 {
                        capture_height as usize
                    } else {
                        capture_height as usize / 2 + capture_height as usize % 2
                    };
                    let plane_len = (stride as usize)
                        .checked_mul(rows)
                        .ok_or_else(|| anyhow!("capture plane size overflow"))?;
                    let map_len = page_offset
                        .checked_add(plane_len)
                        .ok_or_else(|| anyhow!("capture mapping size overflow"))?;
                    if phys_addr == 0 || map_len == 0 {
                        return Err(anyhow!(
                            "capture driver returned invalid plane address or size"
                        ));
                    }

                    let mapped_base = libc::mmap(
                        std::ptr::null_mut(),
                        map_len,
                        libc::PROT_READ,
                        libc::MAP_SHARED,
                        mem_fd,
                        phys_base,
                    );

                    if mapped_base == libc::MAP_FAILED {
                        return Err(anyhow!(
                            "mmap failed for VFB {} Plane {} at physical DMA address 0x{:08X}",
                            vfb_idx,
                            plane_idx,
                            phys_addr
                        ));
                    }

                    let data_ptr = (mapped_base as *mut u8).add(page_offset);
                    planes.push(MappedPlane {
                        data_ptr,
                        mapped_base,
                        map_len,
                        data_len: plane_len,
                    });
                }
                mapped_buffers.push(planes);
            }

            // 8. Start video stream capture
            if fn_start(handle) != 0 {
                return Err(anyhow!("DILE_VT_Start failed"));
            }

            setup.handle = std::ptr::null_mut();

            Ok(Self {
                handle,
                width: capture_width,
                height: capture_height,
                stride,
                pixel_format: prop.pixel_format,
                num_vfbs: cap.num_vfbs as usize,
                num_planes: cap.num_planes as usize,
                mapped_buffers,
                rgb_buffer: vec![0; rgb_size],
                _mem_file: mem_file,
                fn_stop: *fn_stop,
                fn_destroy: *fn_destroy,
                fn_get_current: *fn_get_current,
                fn_wait_vsync: *fn_wait_vsync,
                _lib_pmlog: lib_pmlog,
                _lib_hal: lib_hal,
                _lib_dile: lib,
            })
        }
    }
}

impl ScreenCapture for DileVtCapture {
    fn acquire_frame(&mut self) -> std::result::Result<CapturedFrame<'_>, CaptureError> {
        unsafe {
            // Wait for display vertical blanking
            let ret = (self.fn_wait_vsync)(self.handle);
            if ret != 0 {
                return Err(CaptureError::Unavailable(NativeCaptureError {
                    backend: "DILE_VT",
                    operation: "DILE_VT_WaitVsync",
                    code: ret,
                }));
            }

            // Query active hardware write buffer index
            let mut current_idx: u32 = 0;
            let ret = (self.fn_get_current)(self.handle, std::ptr::null_mut(), &mut current_idx);
            if ret != 0 {
                return Err(CaptureError::Unavailable(NativeCaptureError {
                    backend: "DILE_VT",
                    operation: "DILE_VT_GetCurrentVideoFrameBufferProperty",
                    code: ret,
                }));
            }

            let vfb_idx = (current_idx as usize) % self.num_vfbs;
            let planes = &self.mapped_buffers[vfb_idx];

            match self.pixel_format {
                format if format == DileVtPixelFormat::Yuv420SemiPlanar as u32 => {
                    let (y_len, uv_len, _) =
                        super::vtcapture::nv12_lengths(self.width, self.height, self.stride)?;
                    if planes.len() < 2 || planes[0].data_len < y_len || planes[1].data_len < uv_len
                    {
                        return Err(anyhow!("mapped NV12 capture planes are too short").into());
                    }
                    // SAFETY: these pointers refer to live mmap regions with checked lengths.
                    let y = std::slice::from_raw_parts(planes[0].data_ptr, y_len);
                    let uv = std::slice::from_raw_parts(planes[1].data_ptr, uv_len);
                    super::vtcapture::convert_nv12_to_rgba(
                        self.width,
                        self.height,
                        self.stride,
                        y,
                        uv,
                        &mut self.rgb_buffer,
                    )?;
                }
                format if format == DileVtPixelFormat::Rgb as u32 => {
                    let row_bytes = self.width as usize * 4;
                    let required = (self.height as usize - 1) * self.stride as usize + row_bytes;
                    if planes[0].data_len < required {
                        return Err(anyhow!("mapped RGB capture plane is too short").into());
                    }
                    // SAFETY: the mapped plane covers the checked source length and does not alias the output.
                    let src = std::slice::from_raw_parts(planes[0].data_ptr, required);
                    for (row, dst) in self.rgb_buffer.chunks_exact_mut(row_bytes).enumerate() {
                        let offset = row * self.stride as usize;
                        dst.copy_from_slice(&src[offset..offset + row_bytes]);
                    }
                }
                _ => {
                    // Fallback to Y plane as grayscale if unexpected format
                    let (y_len, _, _) =
                        super::vtcapture::nv12_lengths(self.width, self.height, self.stride)?;
                    if planes[0].data_len < y_len {
                        return Err(anyhow!("mapped capture plane is too short").into());
                    }
                    // SAFETY: the mapped plane covers the checked luma length.
                    let y_plane = std::slice::from_raw_parts(planes[0].data_ptr, y_len);
                    let width = self.width as usize;
                    let height = self.height as usize;
                    let stride = self.stride as usize;
                    for y in 0..height {
                        for x in 0..width {
                            let y_val = y_plane[y * stride + x];
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
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_real_hardware(&self) -> bool {
        true
    }
}

impl Drop for DileVtCapture {
    fn drop(&mut self) {
        unsafe {
            (self.fn_stop)(self.handle);
        }
        self.mapped_buffers.clear();
        unsafe { (self.fn_destroy)(self.handle) };
    }
}

// -----------------------------------------------------------------------------
// Factory & Helper Functions
// -----------------------------------------------------------------------------

#[allow(dead_code)]
pub fn create_capture(width: u32, height: u32) -> Box<dyn ScreenCapture> {
    match DileVtCapture::try_new(width, height, 0) {
        Ok(capture) => Box::new(capture),
        Err(e) => {
            warn!(
                "DileVtCapture not available on this platform ({}). Falling back to MockCapture.",
                e
            );
            Box::new(MockCapture::new(width, height))
        }
    }
}

pub fn detect_source_fps() -> Option<f64> {
    if std::path::Path::new("/usr/bin/luna-send").exists() {
        if let Ok(output) = std::process::Command::new("/usr/bin/luna-send")
            .args([
                "-n",
                "1",
                "luna://com.webos.service.tv.display/getVideoInfo",
                "{}",
            ])
            .output()
        {
            if output.status.success() {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    return source_fps_from_video_info(&val);
                }
            }
        }
    }
    None
}

fn source_fps_from_video_info(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Object(fields) => fields
            .get("frameRate")
            .and_then(parse_frame_rate)
            .or_else(|| fields.values().find_map(source_fps_from_video_info)),
        serde_json::Value::Array(values) => values.iter().find_map(source_fps_from_video_info),
        _ => None,
    }
}

fn parse_frame_rate(value: &serde_json::Value) -> Option<f64> {
    if let Some(value) = value.as_f64() {
        return value.is_finite().then_some(value);
    }
    let value = value.as_str()?.trim();
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator.trim().parse::<f64>().ok()?;
        let denominator = denominator.trim().parse::<f64>().ok()?;
        return (denominator != 0.0).then_some(numerator / denominator);
    }
    value.parse::<f64>().ok().filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_fractional_video_frame_rate() {
        let video_info = serde_json::json!({"payload": {"frameRate": "24000/1001"}});
        assert!((source_fps_from_video_info(&video_info).unwrap() - 23.976).abs() < 0.001);
    }
}
