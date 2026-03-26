use anyhow::Result;
use serde_json::json;
use std::time::Instant;
use wasm_encoder::{
    CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction, Module,
    TypeSection, ValType,
};
use wasmtime::{Engine, Instance, Module as WasmModule, Store};

fn build_bench_module() -> Vec<u8> {
    let mut module = Module::new();

    let mut types = TypeSection::new();
    let fn_type = types.len();
    types
        .ty()
        .function(Vec::<ValType>::new(), vec![ValType::I32]);
    module.section(&types);

    let mut funcs = FunctionSection::new();
    funcs.function(fn_type);
    module.section(&funcs);

    let mut exports = ExportSection::new();
    exports.export("bench_noop", ExportKind::Func, 0);
    module.section(&exports);

    let mut code = CodeSection::new();
    let mut func = Function::new(Vec::new());
    func.instruction(&Instruction::I32Const(1));
    func.instruction(&Instruction::End);
    code.function(&func);
    module.section(&code);

    module.finish()
}

fn bench_reuse_instance(
    engine: &Engine,
    wasm: &[u8],
    iters: u64,
    warmup: u64,
) -> Result<(u128, i32)> {
    let module = WasmModule::new(engine, wasm)?;
    let mut store = Store::new(engine, ());
    let instance = Instance::new(&mut store, &module, &[])?;
    let func = instance.get_typed_func::<(), i32>(&mut store, "bench_noop")?;

    for _ in 0..warmup {
        let _ = func.call(&mut store, ())?;
    }

    let started = Instant::now();
    let mut last = 0i32;
    for _ in 0..iters {
        last = func.call(&mut store, ())?;
    }
    let elapsed = started.elapsed().as_nanos();
    Ok((elapsed, last))
}

fn bench_instantiate_per_call(
    engine: &Engine,
    wasm: &[u8],
    iters: u64,
    warmup: u64,
) -> Result<(u128, i32)> {
    let module = WasmModule::new(engine, wasm)?;

    for _ in 0..warmup {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, &module, &[])?;
        let func = instance.get_typed_func::<(), i32>(&mut store, "bench_noop")?;
        let _ = func.call(&mut store, ())?;
    }

    let started = Instant::now();
    let mut last = 0i32;
    for _ in 0..iters {
        let mut store = Store::new(engine, ());
        let instance = Instance::new(&mut store, &module, &[])?;
        let func = instance.get_typed_func::<(), i32>(&mut store, "bench_noop")?;
        last = func.call(&mut store, ())?;
    }
    let elapsed = started.elapsed().as_nanos();
    Ok((elapsed, last))
}

fn main() -> Result<()> {
    let mut iters = 50_000u64;
    let mut warmup = 2_000u64;
    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--iters=") {
            iters = value.parse()?;
        } else if let Some(value) = arg.strip_prefix("--warmup=") {
            warmup = value.parse()?;
        }
    }

    let wasm = build_bench_module();
    let engine = Engine::default();

    let (reuse_ns, reuse_last) = bench_reuse_instance(&engine, &wasm, iters, warmup)?;
    let (inst_ns, inst_last) = bench_instantiate_per_call(&engine, &wasm, iters, warmup)?;

    let report = json!({
        "iters": iters,
        "warmup": warmup,
        "cases": [
            {
                "name": "reuse_instance",
                "elapsed_ns": reuse_ns,
                "ns_per_iter": (reuse_ns as f64) / (iters.max(1) as f64),
                "last_return": reuse_last,
            },
            {
                "name": "instantiate_per_call",
                "elapsed_ns": inst_ns,
                "ns_per_iter": (inst_ns as f64) / (iters.max(1) as f64),
                "last_return": inst_last,
            },
        ],
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
