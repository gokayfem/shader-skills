use monty::MontyRepl;
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceTracker};
fn run(name: &str, code: &str) {
    let mut repl = MontyRepl::new("p.py", ResourceTracker::default(), CompileOptions::default());
    match repl.feed_run(code, Vec::<(String, MontyObject)>::new(), PrintWriter::Disabled) {
        Ok(v) => println!("{name}: ok -> {v}"), Err(e) => println!("{name}: ERROR {}", e.to_string().lines().last().unwrap_or("")) }
}
fn main() {
    run("while", "i = 0\nwhile i < 3:\n    i += 1\ni");
    run("class", "class A:\n    def __init__(self, v):\n        self.v = v\n    def get(self):\n        return self.v\nA(4).get()");
    run("dataclass", "from dataclasses import dataclass\n@dataclass\nclass P:\n    x: float\nP(1.5).x");
    run("try", "try:\n    1 / 0\nexcept ZeroDivisionError:\n    r = 'caught'\nr");
    run("global", "x = 1\ndef f():\n    global x\n    x = 2\nf()\nx");
    run("generator", "def g():\n    yield 1\n    yield 2\nsum(g())");
    run("math", "import math\nmath.sqrt(16.0)");
}
