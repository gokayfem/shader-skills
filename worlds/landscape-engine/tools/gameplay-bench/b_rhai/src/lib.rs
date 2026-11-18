include!("../../common.rs");
use rhai::{CallFnOptions, Engine, Scope};
use std::cell::Cell;
use std::rc::Rc;
const CODE: &str = r#"
fn noop() { }
fn tick(dt) { let x = get_x(); set_x(x + dt); }
fn work(n) { let s = 0.0; for i in 0..n { s += i.to_float() * 0.5; } s }
"#;
pub fn bench() -> String {
    let x = Rc::new(Cell::new(0.0f64));
    let t0 = now_ms();
    let mut engine = Engine::new();
    let (a, b) = (x.clone(), x.clone());
    engine.register_fn("get_x", move || a.get());
    engine.register_fn("set_x", move |v: f64| b.set(v));
    let ast = engine.compile(CODE).unwrap();
    let startup = now_ms() - t0;
    let mut scope = Scope::new();
    let opts = || CallFnOptions::new().eval_ast(false).rewind_scope(true);
    let noop = best(3, || for _ in 0..CALLS { engine.call_fn_with_options::<()>(opts(), &mut scope, &ast, "noop", ()).unwrap(); });
    let tick = best(3, || for _ in 0..CALLS { engine.call_fn_with_options::<()>(opts(), &mut scope, &ast, "tick", (0.016f64,)).unwrap(); });
    let lp = best(3, || { let _: f64 = engine.call_fn_with_options(opts(), &mut scope, &ast, "work", (LOOP,)).unwrap(); });
    assert!(x.get() > 1.0);
    format!("rhai: startup {startup:.3} ms, noop {:.0} ns/call, tick(2 host calls) {:.0} ns/call, loop {:.1} ns/iter",
        noop * 1e6 / CALLS as f64, tick * 1e6 / CALLS as f64, lp * 1e6 / LOOP as f64)
}
