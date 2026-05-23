#![no_std]
#![allow(unused)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

// Simple bump allocator for WASM linear memory
// We use a static area starting at 0x10000
const ARENA_START: usize = 0x10000;
const ARENA_SIZE: usize = 0x10000; // 64KB

#[global_allocator]
static ALLOC: bump_allocator::BumpAlloc = bump_allocator::BumpAlloc::new(ARENA_START);

mod bump_allocator {
    use core::alloc::{GlobalAlloc, Layout};
    use core::sync::atomic::{AtomicUsize, Ordering};

    pub struct BumpAlloc {
        offset: AtomicUsize,
    }

    impl BumpAlloc {
        pub const fn new(start: usize) -> Self {
            Self { offset: AtomicUsize::new(start) }
        }
    }

    unsafe impl GlobalAlloc for BumpAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let size = layout.size();
            let align = layout.align();
            let old = self.offset.load(Ordering::Relaxed);
            let new = (old + align - 1) & !(align - 1);
            let next = new + size;
            if next > 0x20000 { return core::ptr::null_mut(); }
            self.offset.store(next, Ordering::Relaxed);
            new as *mut u8
        }
        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }
}

/// Read a string from WASM memory
/// In WASM, we use an import to get memory access
#[link(wasm_import_module = "env")]
extern "C" {
    fn memory() -> *mut u8;
}

unsafe fn read_str(ptr: usize, len: usize) -> String {
    let mem = memory();
    let slice = core::slice::from_raw_parts(mem.add(ptr), len);
    core::str::from_utf8_unchecked(slice).into()
}

unsafe fn write_output(s: &str) -> i64 {
    let mem = memory();
    let out_ptr: usize = 0x20000;
    let bytes = s.as_bytes();
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), mem.add(out_ptr), bytes.len());
    ((out_ptr as u64) << 32 | bytes.len() as u64) as i64
}

#[no_mangle]
pub unsafe extern "C" fn str_concat(a_ptr: i32, a_len: i32, b_ptr: i32, b_len: i32) -> i64 {
    let a = read_str(a_ptr as usize, a_len as usize);
    let b = read_str(b_ptr as usize, b_len as usize);
    let result = format!("{}{}", a, b);
    write_output(&result)
}

#[no_mangle]
pub unsafe extern "C" fn json_get(json_ptr: i32, json_len: i32, key_ptr: i32, key_len: i32) -> i64 {
    let json_str = read_str(json_ptr as usize, json_len as usize);
    let key_str = read_str(key_ptr as usize, key_len as usize);

    match lite_json::parse_json(&json_str) {
        Ok(value) => {
            if let lite_json::JsonValue::Object(pairs) = &value {
                for (k, v) in pairs {
                    let ks: String = k.iter().collect();
                    if ks == key_str {
                        let result = value_to_string(v);
                        return write_output(&result);
                    }
                }
            }
            0 // not found
        }
        Err(_) => 0,
    }
}

fn value_to_string(v: &lite_json::JsonValue) -> String {
    match v {
        lite_json::JsonValue::String(chars) => chars.iter().collect(),
        lite_json::JsonValue::Number(n) => {
            if n.negative {
                if n.fraction_length == 0 && n.exponent == 0 {
                    format!("-{}", n.integer)
                } else {
                    let frac = n.fraction as f64 / 10f64.powi(n.fraction_length as i32);
                    let val = n.integer as f64 + frac;
                    format!("-{}", val)
                }
            } else {
                if n.fraction_length == 0 && n.exponent == 0 {
                    format!("{}", n.integer)
                } else {
                    let frac = n.fraction as f64 / 10f64.powi(n.fraction_length as i32);
                    let val = n.integer as f64 + frac;
                    format!("{}", val)
                }
            }
        }
        lite_json::JsonValue::Boolean(b) => if *b { "true".into() } else { "false".into() },
        lite_json::JsonValue::Null => "null".into(),
        lite_json::JsonValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(value_to_string).collect();
            // Re-add quotes for strings in arrays
            format!("[{}]", items.join(","))
        }
        lite_json::JsonValue::Object(pairs) => {
            let items: Vec<String> = pairs.iter().map(|(k, v)| {
                let ks: String = k.iter().collect();
                format!("\"{}\":{}", ks, value_to_string(v))
            }).collect();
            format!("{{{}}}", items.join(","))
        }
    }
}

