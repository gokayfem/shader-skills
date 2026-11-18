include!("../../common.rs");
use std::cell::Cell;
thread_local! { static X: Cell<f64> = const { Cell::new(0.0) }; }
#[inline(never)]
fn tick(dt: f64) { X.with(|x| x.set(x.get() + dt)); }
pub fn bench() -> String {
    let f: &dyn Fn(f64) = &tick;
    let calls = best(5, || for _ in 0..CALLS { std::hint::black_box(f)(std::hint::black_box(0.016)) });
    let lp = best(5, || { let mut s = 0.0; for i in 0..std::hint::black_box(LOOP) { s += i as f64 * 0.5; } std::hint::black_box(s); });
    format!("base: tick {:.1} ns/call, loop {:.2} ns/iter", calls * 1e6 / CALLS as f64, lp * 1e6 / LOOP as f64)
}
