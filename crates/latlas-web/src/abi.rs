// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: The WebAssembly ABI (no wasm-bindgen): the page calls `la_alloc(len)`, writes a
//! UTF-8 JSON request there, calls `la_call(ptr, len)`, reads the response at the returned
//! `(ptr << 32) | len`, and gives both buffers back with `la_free(ptr, len)`. The only unsafe code
//! of the crate, confined here.
//!
//! Owner: latlas-web.
//! Invariants: every buffer crossing the boundary is a boxed slice leaked with its exact length and
//! reclaimed by `la_free` with that length; a non-UTF-8 request is answered, never trusted.

#![allow(unsafe_code)]

use alloc::boxed::Box;
use alloc::vec;

fn leak(b: Box<[u8]>) -> *mut u8 {
    Box::into_raw(b).cast::<u8>()
}

/// 2026-10-24: Allocate `len` bytes for a request.
#[unsafe(no_mangle)]
pub extern "C" fn la_alloc(len: u32) -> *mut u8 {
    leak(vec![0u8; len as usize].into_boxed_slice())
}

/// 2026-10-24: Free a buffer `la_alloc` or `la_call` returned.
///
/// # Safety
/// `ptr` must come from `la_alloc(len)` or be the pointer half of a `la_call` result with that
/// length, and must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn la_free(ptr: *mut u8, len: u32) {
    // SAFETY: the caller hands back a boxed slice this module leaked with exactly this length.
    unsafe { drop(Box::from_raw(core::ptr::slice_from_raw_parts_mut(ptr, len as usize))) };
}

/// 2026-10-24: Handle the request at `ptr..ptr+len`; returns `(out_ptr << 32) | out_len`.
///
/// # Safety
/// `ptr..ptr+len` must be a buffer from `la_alloc` that the page filled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn la_call(ptr: *const u8, len: u32) -> u64 {
    // SAFETY: the page wrote `len` bytes into a buffer of that length from `la_alloc`.
    let req = unsafe { core::slice::from_raw_parts(ptr, len as usize) };
    let resp = match core::str::from_utf8(req) {
        Ok(s) => crate::api::handle(s),
        Err(_) => alloc::string::String::from(
            "{\"error\":\"LatlasToolArgument\",\"message\":\"the request is not UTF-8\"}",
        ),
    };
    let out = resp.into_bytes().into_boxed_slice();
    let n = out.len() as u64;
    ((leak(out) as usize as u64) << 32) | n
}
