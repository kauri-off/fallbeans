# libamd_fidelityfx_vk.so: AMD's FidelityFX API with FSR 3.1 upscaling on Vulkan, for Linux (AMD ships only the
# Windows DLL). FFX_SRC is the FidelityFX SDK v1.1.4 with fidelityfx-sdk-v1.1.4.patch applied (`cargo xtask dist`).
cmake_minimum_required(VERSION 3.21)
project(amd_fidelityfx_vk CXX C)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)
set(CMAKE_CXX_VISIBILITY_PRESET hidden)
set(CMAKE_C_VISIBILITY_PRESET hidden)

if(NOT FFX_SRC)
    message(FATAL_ERROR "FFX_SRC: the patched FidelityFX SDK")
endif()
get_filename_component(FFX_SRC ${FFX_SRC} ABSOLUTE)
set(API ${FFX_SRC}/ffx-api)
set(SDK ${FFX_SRC}/sdk)
set(SC ${SDK}/tools/ffx_shader_compiler)
set(FFX_GPU_PATH ${SDK}/include/FidelityFX/gpu)
set(SHADER_OUT ${CMAKE_BINARY_DIR}/shaders)

find_package(Vulkan REQUIRED)
find_package(Threads REQUIRED)

# AMD's shader compiler (its GLSL path), run at build time.
add_executable(FidelityFX_SC
    ${SC}/src/ffx_sc.cpp ${SC}/src/glsl_compiler.cpp ${SC}/src/utils.cpp
    ${SC}/libs/MD5/md5.cpp ${SC}/libs/SPIRV-Reflect/spirv_reflect.c
    ${SC}/libs/tiny-process-library/process.cpp ${SC}/libs/tiny-process-library/process_unix.cpp)
target_include_directories(FidelityFX_SC PRIVATE ${SC}/src ${SC}/libs/MD5 ${SC}/libs/SPIRV-Reflect ${SC}/libs/tiny-process-library)
target_link_libraries(FidelityFX_SC PRIVATE Threads::Threads)

# FSR 3.1's passes, as AMD builds them for Vulkan (CMakeShadersFSR3Upscaler.txt, CMakeCompileFSR3UpscalerShaders.txt).
# The option lists are quoted: the build's shell would expand {0,1} itself.
include(${FFX_GPU_PATH}/CMakeCompileShaders.txt)
file(MAKE_DIRECTORY ${SHADER_OUT})
set(BASE_ARGS
    -reflection -deps=gcc -DFFX_GPU=1
    -DFFX_FSR3UPSCALER_OPTION_UPSAMPLE_SAMPLERS_USE_DATA_HALF=0
    -DFFX_FSR3UPSCALER_OPTION_ACCUMULATE_SAMPLERS_USE_DATA_HALF=0
    -DFFX_FSR3UPSCALER_OPTION_REPROJECT_SAMPLERS_USE_DATA_HALF=1
    -DFFX_FSR3UPSCALER_OPTION_POSTPROCESSLOCKSTATUS_SAMPLERS_USE_DATA_HALF=0
    -DFFX_FSR3UPSCALER_OPTION_UPSAMPLE_USE_LANCZOS_TYPE=2)
set(API_ARGS -compiler=glslang -e CS --target-env vulkan1.2 -S comp -Os -DFFX_GLSL=1)
set(PERMUTATION_ARGS
    -DFFX_FSR3UPSCALER_OPTION_REPROJECT_USE_LANCZOS_TYPE='{0,1}'
    -DFFX_FSR3UPSCALER_OPTION_HDR_COLOR_INPUT='{0,1}'
    -DFFX_FSR3UPSCALER_OPTION_LOW_RESOLUTION_MOTION_VECTORS='{0,1}'
    -DFFX_FSR3UPSCALER_OPTION_JITTERED_MOTION_VECTORS='{0,1}'
    -DFFX_FSR3UPSCALER_OPTION_INVERTED_DEPTH='{0,1}'
    -DFFX_FSR3UPSCALER_OPTION_APPLY_SHARPENING='{0,1}')
file(GLOB SHADERS ${SDK}/src/backends/vk/shaders/fsr3upscaler/*.glsl)
compile_shaders_with_depfile(FidelityFX_SC "${BASE_ARGS}" "${API_ARGS}" "${PERMUTATION_ARGS}"
    "${FFX_GPU_PATH};${FFX_GPU_PATH}/fsr3upscaler" "${SHADERS}" "${SHADER_OUT}" UNUSED)
set(SHADER_HEADERS)
foreach(SHADER ${SHADERS})
    get_filename_component(NAME ${SHADER} NAME_WLE)
    foreach(VARIANT "" _wave64 _16bit _wave64_16bit)
        list(APPEND SHADER_HEADERS ${SHADER_OUT}/${NAME}${VARIANT}_permutations.h)
    endforeach()
endforeach()
add_custom_target(fsr3upscaler_shaders DEPENDS ${SHADER_HEADERS})

add_library(amd_fidelityfx_vk SHARED
    ${API}/src/ffx_api.cpp
    ${API}/src/ffx_provider.cpp
    ${API}/src/backends.cpp
    ${API}/src/validation.cpp
    ${API}/src/ffx_provider_fsr3upscale.cpp
    ${SDK}/src/components/fsr3upscaler/ffx_fsr3upscaler.cpp
    ${SDK}/src/shared/ffx_assert.cpp
    ${SDK}/src/shared/ffx_message.cpp
    ${SDK}/src/shared/ffx_breadcrumbs_list.cpp
    ${SDK}/src/shared/ffx_object_management.cpp
    ${SDK}/src/backends/vk/ffx_vk.cpp
    ${SDK}/src/backends/shared/ffx_shader_blobs.cpp
    ${SDK}/src/backends/shared/blob_accessors/ffx_fsr3upscaler_shaderblobs.cpp)
add_dependencies(amd_fidelityfx_vk fsr3upscaler_shaders)
target_compile_definitions(amd_fidelityfx_vk PRIVATE
    FFX_BACKEND_VK FFX_FSR3UPSCALER FFX_GCC
    FFXDLL_VERSION_MAJOR=1 FFXDLL_VERSION_MINOR=0 FFXDLL_VERSION_PATCH=1 FFXDLL_VERSION_API=VK
    CI_PIPELINE_ID=0)
target_include_directories(amd_fidelityfx_vk PRIVATE
    ${API}/include ${API}/src ${SDK}/include ${SDK}/src/shared ${SDK}/src/backends/shared
    ${SDK}/src/backends/vk ${SDK}/src/components ${SHADER_OUT})
target_compile_options(amd_fidelityfx_vk PRIVATE -include ${CMAKE_CURRENT_SOURCE_DIR}/compat.h)
target_link_libraries(amd_fidelityfx_vk PRIVATE Vulkan::Vulkan Threads::Threads)
# Only ffx* exported; the C++ runtime inside, so the library loads wherever the game does.
target_link_options(amd_fidelityfx_vk PRIVATE
    -Wl,--no-undefined -Wl,--exclude-libs,ALL -Wl,--version-script=${CMAKE_CURRENT_SOURCE_DIR}/exports.map
    -static-libstdc++ -static-libgcc)
