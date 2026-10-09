//! The window's icon: winit's window class has none, so Windows showed its default (Wayland: the desktop file).

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::window::WindowCreated;
use bevy::winit::WINIT_WINDOWS;

const PNG: &[u8] = include_bytes!("../../../packaging/icons/fallbeans-256.png");

pub struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, set);
    }
}

fn set(mut created: MessageReader<WindowCreated>) {
    for created in created.read() {
        let Some(icon) = icon() else { return };
        WINIT_WINDOWS.with_borrow(|windows| {
            if let Some(window) = windows.get_window(created.window) {
                window.set_window_icon(Some(icon));
            }
        });
    }
}

fn icon() -> Option<winit::window::Icon> {
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
    let (width, height) = (image.width(), image.height());
    winit::window::Icon::from_rgba(image.data?, width, height)
        .inspect_err(|e| warn!("window icon: {e}"))
        .ok()
}
