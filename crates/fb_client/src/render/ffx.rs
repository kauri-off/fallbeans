//! The few pieces of the AMD FidelityFX API (FidelityFX SDK 1.1.4, MIT) that FSR 3.1 upscaling needs, written
//! after its C headers (`ffx_api.h`, `ffx_api_types.h`, `ffx_upscale.h`, `vk/ffx_api_vk.h`): the structures
//! field for field, the constants by value. AMD's signed `amd_fidelityfx_vk.dll` (on Linux the
//! `libamd_fidelityfx_vk.so` xtask builds from the SDK: `packaging/fidelityfx-linux`) is loaded at run time
//! (`libloading`), so nothing of the SDK is needed to build the game, and a game without the library runs on.
#![allow(
    unsafe_code,
    reason = "a C API: its DLL, raw Vulkan handles, pointers into descriptors"
)]

use core::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ash::vk::Handle;
use bevy::prelude::*;
use wgpu::hal::api::Vulkan;
use wgpu_types::TextureFormat;

/// `ffxContext`.
type Context = *mut c_void;
/// `wchar_t`: UTF-16 on Windows, UTF-32 elsewhere.
#[cfg(windows)]
type WChar = u16;
#[cfg(not(windows))]
type WChar = u32;

type PfnCreateContext = unsafe extern "C" fn(*mut Context, *mut Header, *const c_void) -> u32;
type PfnDestroyContext = unsafe extern "C" fn(*mut Context, *const c_void) -> u32;
type PfnQuery = unsafe extern "C" fn(*mut Context, *mut Header) -> u32;
type PfnDispatch = unsafe extern "C" fn(*mut Context, *const Header) -> u32;
/// `ffxApiMessage`.
type PfnMessage = unsafe extern "C" fn(u32, *const WChar);

// Structure types (`ffxStructType_t`).
const CREATE_CONTEXT_UPSCALE: u64 = 0x0001_0000;
const DISPATCH_UPSCALE: u64 = 0x0001_0001;
const QUERY_JITTER_PHASE_COUNT: u64 = 0x0001_0004;
const QUERY_JITTER_OFFSET: u64 = 0x0001_0005;
const CREATE_BACKEND_VK: u64 = 0x3;
const QUERY_PROVIDER_VERSION: u64 = 6;

// `FfxApiCreateContextUpscaleFlags`.
pub const HIGH_DYNAMIC_RANGE: u32 = 1;
pub const DEPTH_INVERTED: u32 = 1 << 3;
pub const DEPTH_INFINITE: u32 = 1 << 4;
pub const AUTO_EXPOSURE: u32 = 1 << 5;
pub const DEBUG_CHECKING: u32 = 1 << 7;

// `FfxApiSurfaceFormat` (the enum's order).
const FORMAT_R32G32B32A32_FLOAT: u32 = 3;
const FORMAT_R16G16B16A16_FLOAT: u32 = 4;
const FORMAT_R32G32_FLOAT: u32 = 6;
const FORMAT_R8G8B8A8_UNORM: u32 = 10;
const FORMAT_R8G8B8A8_SRGB: u32 = 12;
const FORMAT_B8G8R8A8_UNORM: u32 = 14;
const FORMAT_B8G8R8A8_SRGB: u32 = 15;
const FORMAT_R11G11B10_FLOAT: u32 = 16;
const FORMAT_R16G16_FLOAT: u32 = 18;
const FORMAT_R32_FLOAT: u32 = 28;

// `FfxApiResourceType`, `FfxApiResourceUsage`, `FfxApiResourceState`.
const RESOURCE_TEXTURE_2D: u32 = 2;
pub const USAGE_READ_ONLY: u32 = 0;
pub const USAGE_UAV: u32 = 1 << 1;
pub const USAGE_DEPTH_TARGET: u32 = 1 << 2;
/// Sampled: Vulkan's `SHADER_READ_ONLY_OPTIMAL` (wgpu's `TextureUses::RESOURCE` of a colour texture).
pub const STATE_SAMPLED: u32 = (1 << 3) | (1 << 2);
/// `TRANSFER_DST_OPTIMAL` (wgpu's `COPY_DST`): where the prepass's depth copy is anyway.
pub const STATE_COPY_DEST: u32 = 1 << 5;
/// `GENERAL` (wgpu's `STORAGE_READ_WRITE`).
pub const STATE_STORAGE: u32 = 1 << 1;

/// `ffxApiHeader`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
struct Header {
    ty: u64,
    next: *mut Header,
}

impl Header {
    fn of(ty: u64) -> Self {
        Self {
            ty,
            next: core::ptr::null_mut(),
        }
    }
}

/// `FfxApiDimensions2D`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl From<UVec2> for Dimensions {
    fn from(v: UVec2) -> Self {
        Self {
            width: v.x,
            height: v.y,
        }
    }
}

/// `FfxApiFloatCoords2D`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Coords {
    pub x: f32,
    pub y: f32,
}

impl From<Vec2> for Coords {
    fn from(v: Vec2) -> Self {
        Self { x: v.x, y: v.y }
    }
}

/// `FfxApiResourceDescription` (its unions by their texture member).
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ResourceDescription {
    ty: u32,
    format: u32,
    width: u32,
    height: u32,
    depth: u32,
    mip_count: u32,
    flags: u32,
    usage: u32,
}

/// `FfxApiResource`: a `VkImage` and what it is.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Resource {
    resource: *mut c_void,
    description: ResourceDescription,
    state: u32,
}

impl Resource {
    /// None: an optional input left out.
    pub const NONE: Resource = Resource {
        resource: core::ptr::null_mut(),
        description: ResourceDescription {
            ty: 0,
            format: 0,
            width: 0,
            height: 0,
            depth: 0,
            mip_count: 0,
            flags: 0,
            usage: 0,
        },
        state: 0,
    };

    /// A whole 2D texture of one mip level, in `state` (one of the `STATE_` values) before and after the
    /// dispatch, used as `usage` (the `USAGE_` values). None when it is no Vulkan texture or its format is one
    /// FidelityFX does not take.
    pub fn texture(texture: &wgpu::Texture, usage: u32, state: u32) -> Option<Resource> {
        let format = format(texture.format())?;
        // SAFETY: reads the raw image handle only; the texture outlives the dispatch it is recorded in (the
        // view's textures live through the frame), and the guard is dropped on return.
        let hal = unsafe { texture.as_hal::<Vulkan>() }?;
        // SAFETY: as above: the handle is only read.
        let image = unsafe { hal.raw_handle() }.as_raw();
        Some(Resource {
            resource: image as usize as *mut c_void,
            description: ResourceDescription {
                ty: RESOURCE_TEXTURE_2D,
                format,
                width: texture.width(),
                height: texture.height(),
                depth: 1,
                mip_count: 1,
                flags: 0,
                usage,
            },
            state,
        })
    }
}

/// The FidelityFX format of a texture's (`ffxApiGetSurfaceFormatVK`); depth as one float.
pub fn format(f: TextureFormat) -> Option<u32> {
    Some(match f {
        TextureFormat::Rgba32Float => FORMAT_R32G32B32A32_FLOAT,
        TextureFormat::Rgba16Float => FORMAT_R16G16B16A16_FLOAT,
        TextureFormat::Rg32Float => FORMAT_R32G32_FLOAT,
        TextureFormat::Rgba8Unorm => FORMAT_R8G8B8A8_UNORM,
        TextureFormat::Rgba8UnormSrgb => FORMAT_R8G8B8A8_SRGB,
        TextureFormat::Bgra8Unorm => FORMAT_B8G8R8A8_UNORM,
        TextureFormat::Bgra8UnormSrgb => FORMAT_B8G8R8A8_SRGB,
        TextureFormat::Rg11b10Ufloat => FORMAT_R11G11B10_FLOAT,
        TextureFormat::Rg16Float => FORMAT_R16G16_FLOAT,
        TextureFormat::R32Float | TextureFormat::Depth32Float => FORMAT_R32_FLOAT,
        _ => return None,
    })
}

/// `ffxCreateBackendVKDesc`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
struct CreateBackendVk {
    header: Header,
    device: *mut c_void,
    physical_device: *mut c_void,
    device_proc_addr: ash::vk::PFN_vkGetDeviceProcAddr,
}

/// `ffxCreateContextDescUpscale`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
struct CreateUpscale {
    header: Header,
    flags: u32,
    max_render_size: Dimensions,
    max_upscale_size: Dimensions,
    message: Option<PfnMessage>,
}

/// `ffxDispatchDescUpscale`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
pub struct DispatchUpscale {
    header: Header,
    command_list: *mut c_void,
    pub color: Resource,
    pub depth: Resource,
    pub motion_vectors: Resource,
    pub exposure: Resource,
    pub reactive: Resource,
    pub transparency_and_composition: Resource,
    pub output: Resource,
    pub jitter_offset: Coords,
    pub motion_vector_scale: Coords,
    pub render_size: Dimensions,
    pub upscale_size: Dimensions,
    pub enable_sharpening: bool,
    pub sharpness: f32,
    pub frame_time_delta: f32,
    pub pre_exposure: f32,
    pub reset: bool,
    pub camera_near: f32,
    pub camera_far: f32,
    pub camera_fov_angle_vertical: f32,
    pub view_space_to_meters_factor: f32,
    pub flags: u32,
}

impl Default for DispatchUpscale {
    fn default() -> Self {
        Self {
            header: Header::of(DISPATCH_UPSCALE),
            command_list: core::ptr::null_mut(),
            color: Resource::NONE,
            depth: Resource::NONE,
            motion_vectors: Resource::NONE,
            exposure: Resource::NONE,
            reactive: Resource::NONE,
            transparency_and_composition: Resource::NONE,
            output: Resource::NONE,
            jitter_offset: Coords::default(),
            motion_vector_scale: Coords::default(),
            render_size: Dimensions::default(),
            upscale_size: Dimensions::default(),
            enable_sharpening: false,
            sharpness: 0.0,
            frame_time_delta: 0.0,
            pre_exposure: 1.0,
            reset: false,
            camera_near: 0.0,
            camera_far: 0.0,
            camera_fov_angle_vertical: 0.0,
            view_space_to_meters_factor: 1.0,
            flags: 0,
        }
    }
}

/// `ffxQueryDescUpscaleGetJitterPhaseCount`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
struct QueryJitterPhaseCount {
    header: Header,
    render_width: u32,
    display_width: u32,
    out_phase_count: *mut i32,
}

/// `ffxQueryDescUpscaleGetJitterOffset`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
struct QueryJitterOffset {
    header: Header,
    index: i32,
    phase_count: i32,
    out_x: *mut f32,
    out_y: *mut f32,
}

/// `ffxQueryGetProviderVersion`.
#[allow(dead_code, reason = "fields FidelityFX reads, not Rust")]
#[repr(C)]
struct QueryProviderVersion {
    header: Header,
    version_id: u64,
    version_name: *const c_char,
}

/// The DLL's file name.
#[cfg(windows)]
const DLL: &str = "amd_fidelityfx_vk.dll";
#[cfg(not(windows))]
const DLL: &str = "libamd_fidelityfx_vk.so";

/// The loaded DLL and its entry points.
pub struct Api {
    create: PfnCreateContext,
    destroy: PfnDestroyContext,
    query: PfnQuery,
    dispatch: PfnDispatch,
    /// (Last: dropped after nothing can call into it any more.)
    _lib: libloading::Library,
}

impl Api {
    /// Where the DLL may be: beside the game, then in the SDK `FFX_SDK` names (local builds).
    fn places() -> Vec<PathBuf> {
        let mut out = Vec::new();
        let exe = std::env::current_exe().ok();
        if let Some(dir) = exe.as_deref().and_then(Path::parent) {
            out.push(dir.join(DLL));
        }
        if let Some(sdk) = std::env::var_os("FFX_SDK") {
            out.push(PathBuf::from(sdk).join("PrebuiltSignedDLL").join(DLL));
        }
        out
    }

    /// The first DLL found that has the entry points.
    pub fn load() -> Result<Api, String> {
        let mut why = format!("no {DLL} beside the game");
        for path in Self::places().into_iter().filter(|p| p.is_file()) {
            match Self::open(&path) {
                Ok(api) => {
                    info!("upscaling: FidelityFX from {}", path.display());
                    return Ok(api);
                }
                Err(e) => why = format!("{}: {e}", path.display()),
            }
        }
        Err(why)
    }

    fn open(path: &Path) -> Result<Api, libloading::Error> {
        // SAFETY: AMD's signed FidelityFX DLL: loading it runs its initialisers, which have no preconditions.
        let lib = unsafe { libloading::Library::new(path) }?;
        // SAFETY: the symbols are `ffx_api.h`'s, of exactly these types; the pointers are copied out while the
        // library stays loaded in `_lib` for as long as they can be called.
        unsafe {
            let create = *lib.get::<PfnCreateContext>(b"ffxCreateContext\0")?;
            let destroy = *lib.get::<PfnDestroyContext>(b"ffxDestroyContext\0")?;
            let query = *lib.get::<PfnQuery>(b"ffxQuery\0")?;
            let dispatch = *lib.get::<PfnDispatch>(b"ffxDispatch\0")?;
            Ok(Api {
                create,
                destroy,
                query,
                dispatch,
                _lib: lib,
            })
        }
    }
}

/// FidelityFX's messages (with `DEBUG_CHECKING`, and its errors) into the log.
unsafe extern "C" fn message(kind: u32, text: *const WChar) {
    if text.is_null() {
        return;
    }
    let mut units = Vec::new();
    // SAFETY: FidelityFX passes a NUL-terminated wide string that lives through the call; read up to the NUL
    // (at most 4096 units, in case it has none).
    unsafe {
        let mut p = text;
        while *p != 0 && units.len() < 4096 {
            units.push(*p);
            p = p.add(1);
        }
    }
    #[cfg(windows)]
    let s = String::from_utf16_lossy(&units);
    #[cfg(not(windows))]
    let s: String = units.iter().filter_map(|&u| char::from_u32(u)).collect();
    if kind == 0 {
        error!("upscaling: FSR 3.1: {s}");
    } else {
        warn!("upscaling: FSR 3.1: {s}");
    }
}

/// The device's own `vkGetDeviceProcAddr`, behind `device_proc_addr`.
static DEVICE_PROC_ADDR: OnceLock<ash::vk::PFN_vkGetDeviceProcAddr> = OnceLock::new();

/// Does nothing, and says it succeeded (`VK_SUCCESS`): a function FidelityFX looks up that the device lacks.
/// (Called through other signatures: on x86-64 the caller cleans up, and the arguments are ignored.)
unsafe extern "system" fn nothing() -> i32 {
    0
}

/// `vkGetDeviceProcAddr` for FidelityFX. Its backend looks up `vkGetBufferMemoryRequirements2KHR` and the
/// debug-utils functions through the device and calls what it gets; wgpu's device (Vulkan 1.1 and up) has the
/// first in the core under its plain name, without the extension, and the others only with validation: a null
/// pointer, a crash in `ffxCreateContext`. A `KHR` name missing is looked up without the suffix, and a
/// function still missing does nothing.
unsafe extern "system" fn device_proc_addr(
    device: ash::vk::Device,
    name: *const c_char,
) -> ash::vk::PFN_vkVoidFunction {
    let real = *DEVICE_PROC_ADDR.get()?;
    // SAFETY: the device and a NUL-terminated name, as FidelityFX passes them to `vkGetDeviceProcAddr`.
    let found = unsafe { real(device, name) };
    if found.is_some() {
        return found;
    }
    // SAFETY: as above, the name is NUL-terminated.
    let wanted = unsafe { CStr::from_ptr(name) };
    if let Some(core) = wanted.to_bytes().strip_suffix(b"KHR") {
        let plain = [core, b"\0"].concat();
        // SAFETY: the same device and a NUL-terminated name.
        let found = unsafe { real(device, plain.as_ptr().cast()) };
        if found.is_some() {
            return found;
        }
    }
    debug!("upscaling: FSR 3.1 looked up {wanted:?}, which the device has not: it does nothing");
    // SAFETY: only ever called through a pointer that FidelityFX casts to the function's own type (above).
    Some(unsafe { core::mem::transmute::<unsafe extern "system" fn() -> i32, unsafe extern "system" fn()>(nothing) })
}

/// An FSR 3.1 upscaling context for one output size.
pub struct Upscaler {
    api: Arc<Api>,
    ctx: Context,
    device: wgpu::Device,
    /// The jitter sequence's length for the render size made with.
    pub phases: i32,
    pub render: UVec2,
    pub out: UVec2,
}

// SAFETY: the context is a handle into the DLL, used from one thread at a time (the render world's systems, under
// a `Mutex`); FidelityFX keeps no thread-local state for it.
unsafe impl Send for Upscaler {}

impl Upscaler {
    /// A context upscaling `render` (at most) to `out` on wgpu's Vulkan device, with the `flags` of
    /// `ffxCreateContextDescUpscale`.
    pub fn new(api: Arc<Api>, device: &wgpu::Device, render: UVec2, out: UVec2, flags: u32) -> Result<Self, String> {
        // SAFETY: only reads wgpu-hal's handles of the device; they stay valid while `device` (kept in the
        // context) lives, and FidelityFX does not destroy them.
        let mut backend = unsafe {
            let hal = device.as_hal::<Vulkan>().ok_or("not a Vulkan device")?;
            DEVICE_PROC_ADDR.get_or_init(|| hal.shared_instance().raw_instance().fp_v1_0().get_device_proc_addr);
            CreateBackendVk {
                header: Header::of(CREATE_BACKEND_VK),
                device: hal.raw_device().handle().as_raw() as usize as *mut c_void,
                physical_device: hal.raw_physical_device().as_raw() as usize as *mut c_void,
                device_proc_addr,
            }
        };
        let mut desc = CreateUpscale {
            header: Header {
                ty: CREATE_CONTEXT_UPSCALE,
                next: &mut backend.header,
            },
            flags,
            max_render_size: render.into(),
            max_upscale_size: out.into(),
            message: Some(message),
        };
        let mut ctx: Context = core::ptr::null_mut();
        // SAFETY: the descriptors are `ffx_upscale.h`'s and `ffx_api_vk.h`'s, chained through `next`, alive
        // through the call; no allocation callbacks (FidelityFX's own allocator).
        let code = unsafe { (api.create)(&mut ctx, &mut desc.header, core::ptr::null()) };
        if code != 0 || ctx.is_null() {
            return Err(format!("ffxCreateContext: {}", code_name(code)));
        }
        let mut up = Upscaler {
            api,
            ctx,
            device: device.clone(),
            phases: 0,
            render,
            out,
        };
        let mut phases = 0i32;
        let mut q = QueryJitterPhaseCount {
            header: Header::of(QUERY_JITTER_PHASE_COUNT),
            render_width: render.x,
            display_width: out.x,
            out_phase_count: &mut phases,
        };
        // SAFETY: a live context and a query descriptor with a pointer to a local, alive through the call.
        let code = unsafe { (up.api.query)(&mut up.ctx, &mut q.header) };
        up.phases = if code == 0 && phases > 0 {
            phases
        } else {
            phase_count(render.x, out.x)
        };
        Ok(up)
    }

    /// The provider's version, as FidelityFX names it ("FSR 3.1.4").
    pub fn version(&mut self) -> Option<String> {
        let mut q = QueryProviderVersion {
            header: Header::of(QUERY_PROVIDER_VERSION),
            version_id: 0,
            version_name: core::ptr::null(),
        };
        // SAFETY: a live context; the name it returns is a static string of the DLL, read at once.
        unsafe {
            if (self.api.query)(&mut self.ctx, &mut q.header) != 0 || q.version_name.is_null() {
                return None;
            }
            let name = core::ffi::CStr::from_ptr(q.version_name);
            Some(name.to_string_lossy().into_owned())
        }
    }

    /// The sub-pixel jitter of frame `index` of the sequence, in render pixels (`jitterOffset`: what the
    /// projection is offset by, +y down).
    pub fn jitter(&mut self, index: u32) -> Vec2 {
        let i = (index % self.phases.max(1) as u32) as i32;
        let (mut x, mut y) = (0f32, 0f32);
        let mut q = QueryJitterOffset {
            header: Header::of(QUERY_JITTER_OFFSET),
            index: i,
            phase_count: self.phases,
            out_x: &mut x,
            out_y: &mut y,
        };
        // SAFETY: a live context and a query descriptor with pointers to locals, alive through the call.
        let code = unsafe { (self.api.query)(&mut self.ctx, &mut q.header) };
        if code == 0 {
            Vec2::new(x, y)
        } else {
            jitter(i, self.phases)
        }
    }

    /// Records the upscale into `command_buffer` (a `VkCommandBuffer` being recorded).
    ///
    /// # Safety
    /// `command_buffer` must be a Vulkan command buffer of this context's device in the recording state, and every
    /// resource of `d` an image of that device in the state it names, alive until that command buffer has run.
    pub unsafe fn dispatch(
        &mut self,
        command_buffer: ash::vk::CommandBuffer,
        d: &mut DispatchUpscale,
    ) -> Result<(), String> {
        d.header = Header::of(DISPATCH_UPSCALE);
        d.command_list = command_buffer.as_raw() as usize as *mut c_void;
        // SAFETY: as the caller promises; the descriptor is `ffx_upscale.h`'s and lives through the call.
        let code = unsafe { (self.api.dispatch)(&mut self.ctx, &d.header) };
        if code == 0 {
            Ok(())
        } else {
            Err(format!("ffxDispatch: {}", code_name(code)))
        }
    }
}

impl Drop for Upscaler {
    fn drop(&mut self) {
        // SAFETY: the GPU may still run work recorded with the context: the device waits for it before the
        // context (and its images and pipelines) goes. The handles are wgpu-hal's, alive with `device`.
        unsafe {
            if let Some(hal) = self.device.as_hal::<Vulkan>() {
                let _ = hal.raw_device().device_wait_idle();
            }
            (self.api.destroy)(&mut self.ctx, core::ptr::null());
        }
    }
}

/// `FfxApiReturnCodes` by name.
fn code_name(code: u32) -> String {
    match code {
        1 => "error".into(),
        2 => "unknown descriptor type".into(),
        3 => "runtime error".into(),
        4 => "no provider".into(),
        5 => "out of memory".into(),
        6 => "bad parameter".into(),
        c => format!("code {c}"),
    }
}

/// FSR's jitter sequence length for a scale (`ffxFsr3UpscalerGetJitterPhaseCount`): 8 × the area ratio.
pub fn phase_count(render_width: u32, display_width: u32) -> i32 {
    let ratio = display_width as f32 / render_width.max(1) as f32;
    ((8.0 * ratio * ratio) as i32).max(1)
}

/// FSR's jitter of `index` (`ffxFsr3UpscalerGetJitterOffset`): Halton (2, 3) from its second point, centred
/// on the pixel.
pub fn jitter(index: i32, phases: i32) -> Vec2 {
    let i = index.rem_euclid(phases.max(1)) + 1;
    Vec2::new(halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

fn halton(mut index: i32, base: i32) -> f32 {
    let mut f = 1.0;
    let mut r = 0.0;
    while index > 0 {
        f /= base as f32;
        r += f * (index % base) as f32;
        index /= base;
    }
    r
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn layouts_match_the_headers() {
        // (x86_64: pointers of 8 bytes, C's bool of 1.)
        assert_eq!(size_of::<Header>(), 16);
        assert_eq!(size_of::<ResourceDescription>(), 32);
        assert_eq!(size_of::<Resource>(), 48);
        assert_eq!(size_of::<CreateBackendVk>(), 40);
        assert_eq!(size_of::<CreateUpscale>(), 48);
        // Header 16, command list 8, seven resources 336, two coords 16, two dimensions 16, then bool (+3),
        // 3 × f32, bool (+3), 4 × f32, u32: 40.
        assert_eq!(size_of::<DispatchUpscale>(), 16 + 8 + 7 * 48 + 16 + 16 + 40);
        assert_eq!(offset_of!(DispatchUpscale, jitter_offset), 16 + 8 + 7 * 48);
        assert_eq!(offset_of!(DispatchUpscale, sharpness), 16 + 8 + 7 * 48 + 32 + 4);
        assert_eq!(offset_of!(DispatchUpscale, flags), 16 + 8 + 7 * 48 + 32 + 36);
        assert_eq!(size_of::<QueryJitterPhaseCount>(), 32);
        assert_eq!(size_of::<QueryJitterOffset>(), 40);
    }

    #[test]
    fn the_jitter_sequence_is_fsrs() {
        // 1600 → 1232 (0.77): 8 × 1.3² ≈ 13.5 → 13 phases.
        assert_eq!(phase_count(1232, 1600), 13);
        assert_eq!(phase_count(1280, 1920), 18);
        // Halton (2, 3) from index 1: (1/2, 1/3), (1/4, 2/3), (3/4, 1/9), centred.
        let near = |a: Vec2, b: Vec2| (a - b).abs().max_element() < 1e-6;
        assert!(near(jitter(0, 13), Vec2::new(0.0, 1.0 / 3.0 - 0.5)));
        assert!(near(jitter(1, 13), Vec2::new(-0.25, 2.0 / 3.0 - 0.5)));
        assert!(near(jitter(2, 13), Vec2::new(0.25, 1.0 / 9.0 - 0.5)));
        // The sequence repeats, and every offset stays inside the pixel.
        assert!(near(jitter(13, 13), jitter(0, 13)));
        for i in 0..64 {
            let j = jitter(i, 13);
            assert!(j.abs().max_element() < 0.5, "{j}");
        }
    }

    #[test]
    fn formats() {
        assert_eq!(format(TextureFormat::Rgba16Float), Some(4));
        assert_eq!(format(TextureFormat::Depth32Float), Some(28));
        assert_eq!(format(TextureFormat::Rg16Float), Some(18));
        assert_eq!(format(TextureFormat::Depth24Plus), None);
    }
}
