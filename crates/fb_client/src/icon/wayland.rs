//! Wayland ignores winit's icon: the window's own one through `xdg-toplevel-icon-v1` (KDE), else the desktop file.
#![allow(unsafe_code, reason = "winit's raw wl_display and xdg_toplevel")]

use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::ptr::NonNull;

use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::XdgToplevelIconV1;

struct State;

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore XdgToplevelIconManagerV1);
delegate_noop!(State: XdgToplevelIconV1);

/// `Ok(false)`: the compositor has no `xdg-toplevel-icon-v1`.
pub fn set(display: NonNull<c_void>, toplevel: NonNull<c_void>, rgba: &[u8], size: u32) -> Result<bool, String> {
    // SAFETY: winit's display and toplevel outlive this call; the connection is never dropped (below).
    let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display.as_ptr().cast()) });
    let toplevel = unsafe { ObjectId::from_ptr(XdgToplevel::interface(), toplevel.as_ptr().cast()) }
        .and_then(|id| XdgToplevel::from_id(&conn, id))
        .map_err(|e| e.to_string())?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let Ok(manager) = globals.bind::<XdgToplevelIconManagerV1, _, _>(&qh, 1..=1, ()) else {
        return Ok(false);
    };
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;

    // wl_shm's ARGB8888: little-endian BGRA, premultiplied.
    let argb: Vec<u8> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let a = u16::from(p[3]);
            let m = |c: u8| (u16::from(c) * a / 255) as u8;
            [m(p[2]), m(p[1]), m(p[0]), p[3]]
        })
        .collect();
    let fd = rustix::fs::memfd_create("fallbeans-icon", rustix::fs::MemfdFlags::CLOEXEC).map_err(|e| e.to_string())?;
    let mut file = File::from(fd);
    file.write_all(&argb).map_err(|e| e.to_string())?;
    let side = size as i32;
    let pool = shm.create_pool(file.as_fd(), argb.len() as i32, &qh, ());
    let buffer = pool.create_buffer(0, side, side, side * 4, wl_shm::Format::Argb8888, &qh, ());
    let icon = manager.create_icon(&qh, ());
    icon.add_buffer(&buffer, 1);
    manager.set_icon(&toplevel, Some(&icon));
    icon.destroy();
    buffer.destroy();
    pool.destroy();
    manager.destroy();
    queue.roundtrip(&mut State).map_err(|e| e.to_string())?;
    // The registry and wl_shm stay attached to this queue: destroying it under them would leave them dangling.
    std::mem::forget(queue);
    Ok(true)
}
