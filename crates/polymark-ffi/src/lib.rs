#![expect(unsafe_code, reason = "the C ABI is raw pointers and exported symbols")]
#![allow(non_camel_case_types)]

use std::alloc::{Layout, alloc, dealloc};
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use polymark_core::{Input, Output};

pub const POLYMARK_OK: u32 = 0;
pub const POLYMARK_INVALID_UTF8: u32 = 1;
pub const POLYMARK_INVALID_ARGUMENT: u32 = 2;
pub const POLYMARK_INTERNAL: u32 = 3;

pub const POLYMARK_PLAIN: u32 = 0;
pub const POLYMARK_MARKDOWN: u32 = 1;
pub const POLYMARK_HTML: u32 = 2;
pub const POLYMARK_RICH: u32 = 3;
pub const POLYMARK_TELEGRAM: u32 = 4;

const INPUTS: [Input; 4] = [Input::Plain, Input::Markdown, Input::Html, Input::Rich];
const OUTPUTS: [Output; 5] = [Output::Plain, Output::Markdown, Output::Html, Output::Rich, Output::Telegram];

#[repr(C)]
#[derive(Debug)]
pub struct polymark_report {
    pub status: u32,
    pub data: *mut u8,
    pub len: usize,
}

/// # Safety
/// `p` is null with `len` zero, or points to `len` readable bytes.
unsafe fn text<'a>(p: *const u8, len: usize) -> Option<&'a str> {
    let bytes = match (p.is_null(), len) {
        (_, 0) => &[][..],
        (true, _) => return None,
        // SAFETY: the caller passes `len` readable bytes at `p`.
        (false, _) => unsafe { std::slice::from_raw_parts(p, len) },
    };

    std::str::from_utf8(bytes).ok()
}

fn contained(report: impl FnOnce() -> String) -> (u32, Option<String>) {
    match catch_unwind(AssertUnwindSafe(report)) {
        Ok(report) => (POLYMARK_OK, Some(report)),
        Err(_) => (POLYMARK_INTERNAL, None),
    }
}

/// # Safety
/// `out` is null or points to a writable `polymark_report`.
unsafe fn answer(out: *mut polymark_report, status: u32, report: Option<String>) {
    let mut result = polymark_report { status, data: ptr::null_mut(), len: 0 };

    if let Some(report) = report {
        let bytes = report.into_bytes().into_boxed_slice();

        result.len = bytes.len();
        result.data = Box::into_raw(bytes).cast::<u8>();
    }

    if !out.is_null() {
        // SAFETY: the caller passes a writable `polymark_report`.
        unsafe { out.write(result) };
    }
}

/// # Safety
/// `input` is null with `input_len` zero, or points to `input_len` readable
/// bytes; `out` points to a writable `polymark_report`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn polymark_transform(
    input: *const u8,
    input_len: usize,
    from: u32,
    to: u32,
    out: *mut polymark_report,
) {
    let from = INPUTS.get(from as usize);
    let to = OUTPUTS.get(to as usize);

    // SAFETY: forwarded from the caller.
    match (unsafe { text(input, input_len) }, from, to) {
        // SAFETY: forwarded from the caller.
        (None, ..) => unsafe { answer(out, POLYMARK_INVALID_UTF8, None) },
        (Some(source), Some(from), Some(to)) => {
            let (status, report) = contained(|| polymark_core::report(&polymark_core::transform(source, *from, *to)));

            // SAFETY: forwarded from the caller.
            unsafe { answer(out, status, report) };
        }
        // SAFETY: forwarded from the caller.
        _ => unsafe { answer(out, POLYMARK_INVALID_ARGUMENT, None) },
    }
}

/// # Safety
/// As for `polymark_transform`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn polymark_clean(input: *const u8, input_len: usize, out: *mut polymark_report) {
    // SAFETY: forwarded from the caller.
    match unsafe { text(input, input_len) } {
        // SAFETY: forwarded from the caller.
        None => unsafe { answer(out, POLYMARK_INVALID_UTF8, None) },
        Some(source) => {
            let (status, report) = contained(|| polymark_core::report_clean(&polymark_core::clean(source)));

            // SAFETY: forwarded from the caller.
            unsafe { answer(out, status, report) };
        }
    }
}

/// # Safety
/// `report` is null or points to a `polymark_report` that this module wrote and
/// that has not been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn polymark_report_free(report: *mut polymark_report) {
    // SAFETY: the caller passes null or a valid `polymark_report`.
    let Some(report) = (unsafe { report.as_mut() }) else { return };

    if !report.data.is_null() {
        let bytes = ptr::slice_from_raw_parts_mut(report.data, report.len);
        // SAFETY: `data` came from `Box::into_raw` of exactly this many bytes.
        drop(unsafe { Box::from_raw(bytes) });
    }

    report.data = ptr::null_mut();
    report.len = 0;
}

#[unsafe(no_mangle)]
pub extern "C" fn polymark_version() -> *const c_char {
    const VERSION: &std::ffi::CStr =
        match std::ffi::CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(version) => version,
            Err(_) => panic!("the version has no NUL"),
        };

    VERSION.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn polymark_alloc(len: usize) -> *mut u8 {
    match Layout::array::<u8>(len) {
        // SAFETY: the layout is valid and not zero-sized.
        Ok(layout) if len > 0 => unsafe { alloc(layout) },
        _ => ptr::null_mut(),
    }
}

/// # Safety
/// `p` is null or came from `polymark_alloc(len)` and has not been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn polymark_dealloc(p: *mut u8, len: usize) {
    if let (false, Ok(layout)) = (p.is_null(), Layout::array::<u8>(len))
        && len > 0
    {
        // SAFETY: `p` came from `polymark_alloc` with this layout.
        unsafe { dealloc(p, layout) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &[u8], from: u32, to: u32) -> (u32, String) {
        let mut out = polymark_report { status: 9, data: ptr::null_mut(), len: 0 };

        // SAFETY: the text and `out` are valid for the call.
        unsafe { polymark_transform(text.as_ptr(), text.len(), from, to, &raw mut out) };

        let report = if out.data.is_null() {
            String::new()
        } else {
            // SAFETY: `polymark_transform` wrote `len` bytes at `data`.
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(out.data, out.len) }).into_owned()
        };

        // SAFETY: `out` was written by `polymark_transform`.
        unsafe { polymark_report_free(&raw mut out) };
        (out.status, report)
    }

    #[test]
    fn reports_through_the_abi() {
        assert_eq!(
            run(b"**a**", 1, 2),
            (POLYMARK_OK, "{\"result\":\"<p><strong>a</strong></p>\",\"warnings\":{}}".into())
        );
        assert_eq!(run(&[0xff], 0, 0).0, POLYMARK_INVALID_UTF8);
        assert_eq!(run(b"a", 0, 9).0, POLYMARK_INVALID_ARGUMENT);
    }

    #[test]
    fn contains_a_panic() {
        assert_eq!(contained(|| panic!("a defect")), (POLYMARK_INTERNAL, None));
    }
}
