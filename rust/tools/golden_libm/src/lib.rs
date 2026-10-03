//! `fb_shared::m` as WebAssembly exports: the TS golden traces compute with the same bits as Rust.
#![allow(unsafe_code, reason = "unmangled exports for WebAssembly")]
use fb_shared::m;

#[unsafe(no_mangle)]
pub extern "C" fn fb_sin(x: f64) -> f64 {
    m::sin(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_cos(x: f64) -> f64 {
    m::cos(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_atan2(y: f64, x: f64) -> f64 {
    m::atan2(y, x)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_atan(x: f64) -> f64 {
    m::atan(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_exp(x: f64) -> f64 {
    m::exp(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_pow(x: f64, y: f64) -> f64 {
    m::pow(x, y)
}
