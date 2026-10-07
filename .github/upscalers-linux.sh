#!/bin/sh
# The Linux packages' upscalers (release.yml, the appimage and flatpak jobs): NVIDIA's DLSS SDK and its Linux
# library, the Vulkan headers, AMD's FidelityFX SDK whose sources `cargo xtask dist` builds for Linux. Checked as
# the Windows job checks them; the paths go to $GITHUB_ENV.
set -eu
here=$(pwd)
sparse() { # repository, tag, directory, paths…
  repo=$1 tag=$2 dir=$3
  shift 3
  git clone --quiet --filter=blob:none --no-checkout --depth 1 --branch "$tag" "$repo" "$dir"
  git -C "$dir" sparse-checkout set --no-cone "$@"
  git -C "$dir" checkout --quiet
}
sparse https://github.com/NVIDIA/DLSS.git "$DLSS_SDK_TAG" dlss-sdk \
  /include/ /lib/Linux_x86_64/libnvsdk_ngx.a /doc/DLSS_Programming_Guide_Release.pdf /LICENSE.txt
test "$(git -C dlss-sdk rev-parse HEAD)" = "$DLSS_SDK_COMMIT"
test -d dlss-sdk/include -a -f dlss-sdk/lib/Linux_x86_64/libnvsdk_ngx.a -a -f dlss-sdk/doc/DLSS_Programming_Guide_Release.pdf
lib="libnvidia-ngx-dlss.so.${DLSS_DLL_TAG#v}"
sparse https://github.com/NVIDIA/DLSS.git "$DLSS_DLL_TAG" dlss-dll "/lib/Linux_x86_64/rel/$lib"
echo "$DLSS_SO_SHA256  dlss-dll/lib/Linux_x86_64/rel/$lib" | sha256sum -c -
git clone --quiet --depth 1 --branch "$VULKAN_HEADERS_TAG" \
  https://github.com/KhronosGroup/Vulkan-Headers.git vulkan-headers
curl -fsSL -o ffx.zip "$FFX_URL"
echo "$FFX_SHA256  ffx.zip" | sha256sum -c -
unzip -q ffx.zip 'ffx-api/*' 'sdk/include/*' 'sdk/src/*' 'sdk/tools/ffx_shader_compiler/*' 'docs/license.md' -d ffx
pdftotext -layout -enc UTF-8 dlss-sdk/doc/DLSS_Programming_Guide_Release.pdf dlss-guide.txt
{
  echo "DLSS_SDK=$here/dlss-sdk"
  echo "DLSS_DLL=$here/dlss-dll/lib/Linux_x86_64/rel/$lib"
  echo "DLSS_GUIDE_TEXT=$here/dlss-guide.txt"
  echo "VULKAN_SDK=$here/vulkan-headers"
  echo "FFX_SDK=$here/ffx"
} >> "$GITHUB_ENV"
