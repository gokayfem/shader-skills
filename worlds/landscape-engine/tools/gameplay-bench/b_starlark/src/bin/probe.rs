use starlark::environment::{Globals, Module};
use starlark::eval::Evaluator;
use starlark::syntax::{AstModule, Dialect};
fn run(name: &str, code: &str) {
    let r = Module::with_temp_heap(|module| {
        let ast = AstModule::parse("p.star", code.to_owned(), &Dialect::Extended).map_err(|e| e.to_string())?;
        let mut eval = Evaluator::new(&module);
        eval.eval_module(ast, &Globals::extended_internal()).map(|v| v.to_string()).map_err(|e| e.to_string())
    });
    match r { Ok(v) => println!("{name}: ok -> {v}"), Err(e) => println!("{name}: ERROR {}", e.lines().next().unwrap_or("")) }
}
fn main() {
    run("while", "def f():\n    i = 0\n    while i < 3:\n        i += 1\n    return i\nf()");
    run("recursion", "def f(n):\n    return 1 if n < 2 else n * f(n - 1)\nf(5)");
    run("mutate global list in fn", "xs = []\ndef f():\n    xs.append(1)\nf()\nxs");
    run("reassign global in fn", "x = 1\ndef f():\n    global x\n    x = 2\nf()\nx");
    run("class", "class A:\n    pass\n1");
    run("try", "def f():\n    try:\n        pass\n    except:\n        pass\n1");
    run("lambda+fstring", "y = 3\nf = lambda a: a * 2\nf'{f(y)}'");
    run("types", "def f(a: int) -> int:\n    return a\nf(2)");
}
