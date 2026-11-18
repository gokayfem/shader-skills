#[cfg(target_arch = "wasm32")]
mod clock {
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance)]
        fn now() -> f64;
    }
    pub fn now_ms() -> f64 { now() }
}
#[cfg(not(target_arch = "wasm32"))]
mod clock {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    pub fn now_ms() -> f64 { START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0 }
}
pub use clock::now_ms;

/// Best of `reps` runs of `f`, in ms.
pub fn best<F: FnMut()>(reps: u32, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..reps { let t = now_ms(); f(); b = b.min(now_ms() - t); }
    b
}

pub const CALLS: u32 = 100_000;
pub const LOOP: i64 = 1_000_000;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run() -> String { crate::bench() }
