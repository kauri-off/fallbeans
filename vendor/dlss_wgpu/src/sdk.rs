use crate::{feature_info::with_feature_info, nvsdk_ngx::*};
use std::{
    ptr,
    sync::{Arc, Mutex},
    thread,
};
use uuid::Uuid;
use wgpu::{Device, hal::api::Vulkan};

/// Application-wide DLSS object.
pub struct DlssSdk {
    pub(crate) parameters: *mut NVSDK_NGX_Parameter,
    pub(crate) device: Device,
}

impl DlssSdk {
    /// Creates the DLSS SDK.
    ///
    /// This should be done once per application.
    pub fn new(project_id: Uuid, device: Device) -> Result<Arc<Mutex<Self>>, DlssError> {
        check_for_updates(project_id);

        let mut parameters = ptr::null_mut();
        unsafe {
            // (fallbeans patch: a device of another API is not supported, not a panic.)
            let Some(hal_device) = device.as_hal::<Vulkan>() else {
                return Err(DlssError::FeatureNotSupported);
            };
            let shared_instance = hal_device.shared_instance();
            let raw_instance = shared_instance.raw_instance();

            with_feature_info(project_id, Default::default(), |feature_info| {
                check_ngx_result(NVSDK_NGX_VULKAN_Init_with_ProjectID(
                    feature_info.Identifier.v.ProjectDesc.ProjectId,
                    NVSDK_NGX_EngineType_NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                    feature_info.Identifier.v.ProjectDesc.EngineVersion,
                    feature_info.ApplicationDataPath,
                    raw_instance.handle(),
                    hal_device.raw_physical_device(),
                    hal_device.raw_device().handle(),
                    shared_instance.entry().static_fn().get_instance_proc_addr,
                    raw_instance.fp_v1_0().get_device_proc_addr,
                    feature_info.FeatureInfo,
                    NVSDK_NGX_Version_NVSDK_NGX_Version_API,
                ))
            })?;

            check_ngx_result(NVSDK_NGX_VULKAN_GetCapabilityParameters(&mut parameters))?;

            let mut dlss_supported = 0;
            let result = check_ngx_result(NVSDK_NGX_Parameter_GetI(
                parameters,
                NVSDK_NGX_Parameter_SuperSampling_Available.as_ptr().cast(),
                &mut dlss_supported,
            ));
            if result.is_err() {
                check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(parameters))?;
                result?;
            }
            if dlss_supported == 0 {
                check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(parameters))?;
                return Err(DlssError::FeatureNotSupported);
            }
        }

        Ok(Arc::new(Mutex::new(Self { parameters, device })))
    }

    /// Returns the number of bytes of VRAM allocated by DLSS.
    pub fn get_vram_allocated_bytes(&mut self) -> Result<u64, DlssError> {
        let mut vram_allocated_bytes = 0;
        check_ngx_result(unsafe {
            NGX_DLSS_GET_STATS(self.parameters, &mut vram_allocated_bytes)
        })?;
        Ok(vram_allocated_bytes)
    }

    /// fallbeans patch: the model ("render preset", `NVSDK_NGX_DLSS_Hint_Render_Preset` in
    /// `nvsdk_ngx_defs.h`) the Super Resolution contexts made afterwards in `mode` use (`Auto`: every mode).
    /// 0 is NVIDIA's default for the mode; 12 and 13 are presets L and M, DLSS 4.5's second-generation
    /// transformer models (DLLs from 310.5.0 on; an older DLL falls back to its default).
    pub fn set_render_preset(&mut self, mode: DlssPerfQualityMode, preset: u32) {
        const DLAA: &std::ffi::CStr = c"DLSS.Hint.Render.Preset.DLAA";
        const QUALITY: &std::ffi::CStr = c"DLSS.Hint.Render.Preset.Quality";
        const BALANCED: &std::ffi::CStr = c"DLSS.Hint.Render.Preset.Balanced";
        const PERFORMANCE: &std::ffi::CStr = c"DLSS.Hint.Render.Preset.Performance";
        const ULTRA_PERFORMANCE: &std::ffi::CStr = c"DLSS.Hint.Render.Preset.UltraPerformance";
        let names: &[&std::ffi::CStr] = match mode {
            DlssPerfQualityMode::Auto => &[DLAA, QUALITY, BALANCED, PERFORMANCE, ULTRA_PERFORMANCE],
            DlssPerfQualityMode::Dlaa => &[DLAA],
            DlssPerfQualityMode::Quality => &[QUALITY],
            DlssPerfQualityMode::Balanced => &[BALANCED],
            DlssPerfQualityMode::Performance => &[PERFORMANCE],
            DlssPerfQualityMode::UltraPerformance => &[ULTRA_PERFORMANCE],
        };
        for name in names {
            // SAFETY: `parameters` is the live capability parameter block of this SDK (destroyed only in
            // `drop`), and the name is a NUL-terminated string NGX copies.
            unsafe { NVSDK_NGX_Parameter_SetUI(self.parameters, name.as_ptr(), preset) };
        }
    }
}

fn check_for_updates(project_id: Uuid) {
    thread::spawn(move || {
        with_feature_info(
            project_id,
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_SuperSampling,
            |feature_info| unsafe {
                NVSDK_NGX_UpdateFeature(&feature_info.Identifier, feature_info.FeatureID);
            },
        );
        with_feature_info(
            project_id,
            NVSDK_NGX_Feature_NVSDK_NGX_Feature_RayReconstruction,
            |feature_info| unsafe {
                NVSDK_NGX_UpdateFeature(&feature_info.Identifier, feature_info.FeatureID);
            },
        );
    });
}

impl Drop for DlssSdk {
    fn drop(&mut self) {
        unsafe {
            let hal_device = self.device.as_hal::<Vulkan>().unwrap();
            hal_device
                .raw_device()
                .device_wait_idle()
                .expect("Failed to wait for idle device when destroying DlssSdk");

            check_ngx_result(NVSDK_NGX_VULKAN_DestroyParameters(self.parameters))
                .expect("Failed to destroy DlssSdk parameters");
            check_ngx_result(NVSDK_NGX_VULKAN_Shutdown1(hal_device.raw_device().handle()))
                .expect("Failed to destroy DlssSdk");
        }
    }
}

unsafe impl Send for DlssSdk {}
unsafe impl Sync for DlssSdk {}
