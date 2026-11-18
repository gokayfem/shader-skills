include!("../../common.rs");
use starlark::environment::{GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::none::NoneType;
use std::cell::Cell;
thread_local! { static X: Cell<f64> = const { Cell::new(0.0) }; }
#[starlark_module]
fn host(builder: &mut GlobalsBuilder) {
    fn get_x() -> anyhow::Result<f64> { Ok(X.with(|x| x.get())) }
    fn set_x(v: starlark::values::float::UnpackFloat) -> anyhow::Result<NoneType> { X.with(|x| x.set(v.0)); Ok(NoneType) }
}
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
    Module::with_temp_heap(|module| {
        let t0 = now_ms();
        let globals = GlobalsBuilder::standard().with(host).build();
        let ast = AstModule::parse("s.star", CODE.to_owned(), &Dialect::Extended).unwrap();
        { let mut eval = Evaluator::new(&module); eval.eval_module(ast, &globals).unwrap(); }
        let startup = now_ms() - t0;
        let noop = module.get("noop").unwrap();
        let tick = module.get("tick").unwrap();
        let work = module.get("work").unwrap();
        let mut eval = Evaluator::new(&module);
        let dt = module.heap().alloc(0.016f64);
        let n = module.heap().alloc(LOOP as i32);
        let tn = best(3, || for _ in 0..CALLS { eval.eval_function(noop, &[], &[]).unwrap(); });
        let tt = best(3, || for _ in 0..CALLS { eval.eval_function(tick, &[dt], &[]).unwrap(); });
        let tl = best(3, || { eval.eval_function(work, &[n], &[]).unwrap(); });
        assert!(X.with(|x| x.get()) > 1.0);
        format!("starlark: startup {startup:.3} ms, noop {:.0} ns/call, tick(2 host calls) {:.0} ns/call, loop {:.1} ns/iter",
            tn * 1e6 / CALLS as f64, tt * 1e6 / CALLS as f64, tl * 1e6 / LOOP as f64)
    })
}
