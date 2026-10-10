//! The window's icon: winit's window class has none, so Windows showed its default (Wayland: `wayland.rs`).

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::NonSendMarker;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::window::WindowCreated;
use bevy::winit::WINIT_WINDOWS;

#[cfg(target_os = "linux")]
mod wayland;

const PNG: &[u8] = include_bytes!("../../../packaging/icons/fallbeans-256.png");

pub struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, set);
    }
}

// `WINIT_WINDOWS` is a thread local of the main thread.
fn set(mut created: MessageReader<WindowCreated>, _main_thread: NonSendMarker) {
    for created in created.read() {
        let Some((rgba, size)) = rgba() else { return };
        WINIT_WINDOWS.with_borrow(|windows| {
            let Some(window) = windows.get_window(created.window) else {
                return;
            };
            let window: &winit::window::Window = window;
            #[cfg(target_os = "linux")]
            if let Some(toplevel) = winit::platform::wayland::WindowExtWayland::xdg_toplevel(window) {
                use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
                if let Ok(RawDisplayHandle::Wayland(display)) = window.display_handle().map(|h| h.as_raw()) {
                    match wayland::set(display.display, toplevel, &rgba, size) {
                        Ok(true) => {}
                        Ok(false) => info!("window icon: the compositor takes it from the desktop file"),
                        Err(e) => warn!("window icon: {e}"),
                    }
                }
                return;
            }
            match winit::window::Icon::from_rgba(rgba, size, size) {
                Ok(icon) => window.set_window_icon(Some(icon)),
                Err(e) => warn!("window icon: {e}"),
            }
        });
    }
}

fn rgba() -> Option<(Vec<u8>, u32)> {
    let image = Image::from_buffer(
        PNG,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .inspect_err(|e| warn!("window icon: {e}"))
    .ok()?;
    let size = image.width();
    Some((image.data?, size))
}
