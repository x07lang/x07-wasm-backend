#![no_std]

extern crate alloc;
extern crate rlibc;

use alloc::vec::Vec;
use core::panic::PanicInfo;

wit_bindgen::generate!({
    world: "x07:web-ui-adapter/web-ui-app-with-solve@0.1.0",
    path: [
        "../../wit/x07/solve/0.1.0",
        "../../wit/x07/web_ui/0.2.0",
        "../../wit/x07/web_ui_adapter/0.1.0",
    ],
    generate_all,
});

#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[no_mangle]
pub unsafe extern "C" fn cabi_realloc(
    old_ptr: *mut u8,
    old_len: usize,
    align: usize,
    new_len: usize,
) -> *mut u8 {
    use alloc::alloc::{Layout, alloc, handle_alloc_error, realloc};

    let layout;
    let ptr = if old_len == 0 {
        if new_len == 0 {
            return align as *mut u8;
        }
        layout = Layout::from_size_align_unchecked(new_len, align);
        alloc(layout)
    } else {
        debug_assert_ne!(new_len, 0, "non-zero old_len requires non-zero new_len");
        layout = Layout::from_size_align_unchecked(old_len, align);
        realloc(old_ptr, layout, new_len)
    };

    if ptr.is_null() {
        handle_alloc_error(layout);
    }
    ptr
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    #[cfg(target_arch = "wasm32")]
    core::arch::wasm32::unreachable();

    #[allow(unreachable_code)]
    loop {}
}

const INIT_DISPATCH: &[u8] =
    br#"{"v":1,"kind":"x07.web_ui.dispatch","state":null,"event":{"type":"init"}}"#;

struct WebUiAdapter;

impl Guest for WebUiAdapter {
    fn init() -> Vec<u8> {
        x07::solve::handler::solve(INIT_DISPATCH)
    }

    fn step(input: Vec<u8>) -> Vec<u8> {
        x07::solve::handler::solve(&input)
    }
}

export!(WebUiAdapter);
