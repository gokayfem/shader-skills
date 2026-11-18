include!("../../common.rs");
use monty::{MontyRepl, ReplProgress};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};
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
    let mut repl = MontyRepl::new("g.py", ResourceTracker::default(), CompileOptions::default());
    let inputs: Vec<(String, MontyObject)> = vec![
        ("get_x".into(), MontyObject::function("get_x", None)),
        ("set_x".into(), MontyObject::function("set_x", None)),
    ];
    repl.feed_run(CODE, inputs, PrintWriter::Disabled).unwrap();
    let startup = now_ms() - t0;
    let tn = best(3, || for _ in 0..CALLS { repl.call_function("noop", Vec::<MontyObject>::new(), PrintWriter::Disabled).unwrap(); });
    let tl = best(3, || { repl.call_function("work", vec![MontyObject::int(LOOP)], PrintWriter::Disabled).unwrap(); });
    // Host -> script -> host: a snippet per tick, suspending at each external call.
    let mut x = 0.0f64;
    let mut slot = Some(repl);
    let calls = CALLS / 10;
    let tt = best(3, || for _ in 0..calls {
        let mut p = slot.take().unwrap().feed_start("tick(0.016)", Vec::<(String, MontyObject)>::new(), PrintWriter::Disabled).map_err(|e| e.error).unwrap();
        loop {
            match p {
                ReplProgress::FunctionCall(call) => {
                    let r = if call.function_name == "get_x" { MontyObject::float(x) } else {
                        x = call.args.arg(0).unwrap().as_float().unwrap(); MontyObject::none() };
                    p = call.resume(r, PrintWriter::Disabled).map_err(|e| e.error).unwrap();
                }
                ReplProgress::Complete { repl, .. } => { slot = Some(repl); break; }
                _ => panic!("unexpected"),
            }
        }
    });
    assert!(x > 0.01);
    format!("monty: startup {startup:.3} ms, noop {:.0} ns/call, tick(2 host calls, via snippet+suspend) {:.0} ns/call, loop {:.1} ns/iter",
        tn * 1e6 / CALLS as f64, tt * 1e6 / calls as f64, tl * 1e6 / LOOP as f64)
}
