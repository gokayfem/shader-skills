include!("../../common.rs");
use rustpython_vm::{Interpreter, Settings};
use std::cell::Cell;
thread_local! { static X: Cell<f64> = const { Cell::new(0.0) }; }
const CODE: &str = r#"
def noop():
    pass
def tick(dt):
    x = get_x()
    set_x(x + dt)
def work(n):
    s = 0.0
    for i in range(n):
        s += i * 0.5
    return s
"#;
pub fn bench() -> String {
    let t0 = now_ms();
    let interp = Interpreter::without_stdlib(Settings::default());
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        scope.globals.set_item("get_x", vm.new_function("get_x", || X.with(|x| x.get())).into(), vm).unwrap();
        scope.globals.set_item("set_x", vm.new_function("set_x", |v: f64| X.with(|x| x.set(v))).into(), vm).unwrap();
        vm.run_code_string(scope.clone(), CODE, "<s>").map_err(|e| { vm.print_exception(&e); }).unwrap();
        let startup = now_ms() - t0;
        let noop = scope.globals.get_item("noop", vm).unwrap();
        let tick = scope.globals.get_item("tick", vm).unwrap();
        let work = scope.globals.get_item("work", vm).unwrap();
        let tn = best(3, || for _ in 0..CALLS { noop.call((), vm).unwrap(); });
        let tt = best(3, || for _ in 0..CALLS { tick.call((0.016f64,), vm).unwrap(); });
        let tl = best(3, || { work.call((LOOP,), vm).unwrap(); });
        assert!(X.with(|x| x.get()) > 1.0);
        format!("rustpython: startup {startup:.3} ms, noop {:.0} ns/call, tick(2 host calls) {:.0} ns/call, loop {:.1} ns/iter",
            tn * 1e6 / CALLS as f64, tt * 1e6 / CALLS as f64, tl * 1e6 / LOOP as f64)
    })
}
