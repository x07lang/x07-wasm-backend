use anyhow::{Context, Result};
use wasmtime::{Memory, Store, TypedFunc};

#[derive(Debug, Clone)]
pub struct LinearMemoryTooSmall {
    pub need_bytes: u64,
    pub have_bytes: u64,
}

impl std::fmt::Display for LinearMemoryTooSmall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "wasm linear memory too small for allocations: need={} bytes have={} bytes",
            self.need_bytes, self.have_bytes
        )
    }
}

impl std::error::Error for LinearMemoryTooSmall {}

#[derive(Debug, Clone)]
pub struct OutputLimitExceeded {
    pub out_len: u32,
    pub max_output_bytes: u32,
}

impl std::fmt::Display for OutputLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "output too large: out_len={} max_output_bytes={}",
            self.out_len, self.max_output_bytes
        )
    }
}

impl std::error::Error for OutputLimitExceeded {}

#[derive(Debug, Clone)]
pub struct SolveV2MemoryLayout {
    pub heap_base: u32,
    pub data_end: u32,
    pub retptr: u32,
    pub input_ptr: u32,
    pub arena_ptr: u32,
}

#[derive(Debug, Clone)]
pub struct SolveV2CallResult {
    pub layout: SolveV2MemoryLayout,
    pub output: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub fn call_solve_v2<T>(
    store: &mut Store<T>,
    memory: &Memory,
    func: &TypedFunc<(i32, i32, i32, i32, i32), ()>,
    heap_base: u32,
    data_end: u32,
    input: &[u8],
    arena_cap_bytes: u32,
    max_output_bytes: u32,
) -> Result<SolveV2CallResult> {
    let mem_size = memory.data_size(&mut *store) as u64;

    let alloc_base = std::cmp::max(heap_base, data_end) as u64;
    let retptr = align_up(alloc_base, 8);
    let input_ptr = align_up(retptr + 8, 8);
    let arena_ptr = align_up(input_ptr + input.len() as u64, 8);

    let arena_end = arena_ptr
        .checked_add(arena_cap_bytes as u64)
        .ok_or_else(|| anyhow::anyhow!("arena_end overflow"))?;
    if arena_end > mem_size {
        return Err(anyhow::Error::new(LinearMemoryTooSmall {
            need_bytes: arena_end,
            have_bytes: mem_size,
        }));
    }

    let retptr_u32 = u32::try_from(retptr).context("retptr overflow")?;
    let input_ptr_u32 = u32::try_from(input_ptr).context("input_ptr overflow")?;
    let arena_ptr_u32 = u32::try_from(arena_ptr).context("arena_ptr overflow")?;

    let retptr_off = usize::try_from(retptr_u32 as u64).context("retptr offset overflow")?;
    let input_ptr_off =
        usize::try_from(input_ptr_u32 as u64).context("input_ptr offset overflow")?;

    memory
        .write(&mut *store, retptr_off, &[0u8; 8])
        .context("write retptr")?;
    memory
        .write(&mut *store, input_ptr_off, input)
        .context("write input")?;

    func.call(
        &mut *store,
        (
            retptr_u32 as i32,
            arena_ptr_u32 as i32,
            arena_cap_bytes as i32,
            input_ptr_u32 as i32,
            (input.len() as u32) as i32,
        ),
    )
    .context("call x07_solve_v2")?;

    let mut ret = [0u8; 8];
    memory
        .read(&mut *store, retptr_off, &mut ret)
        .context("read bytes_t")?;
    let out_ptr = u32::from_le_bytes(ret[0..4].try_into().unwrap());
    let out_len = u32::from_le_bytes(ret[4..8].try_into().unwrap());

    if out_len > max_output_bytes {
        return Err(anyhow::Error::new(OutputLimitExceeded {
            out_len,
            max_output_bytes,
        }));
    }

    let out_start = out_ptr as u64;
    let out_end = out_start
        .checked_add(out_len as u64)
        .ok_or_else(|| anyhow::anyhow!("out_end overflow"))?;
    let arena_start = arena_ptr_u32 as u64;
    let arena_end = arena_start + arena_cap_bytes as u64;
    if out_start < arena_start || out_end > arena_end {
        anyhow::bail!(
            "output bytes not within arena: out=[{}, {}) arena=[{}, {})",
            out_start,
            out_end,
            arena_start,
            arena_end
        );
    }
    if out_end > mem_size {
        anyhow::bail!(
            "output bytes out of bounds: out_end={} mem_size={}",
            out_end,
            mem_size
        );
    }

    let mut out = vec![0u8; out_len as usize];
    let out_off = usize::try_from(out_ptr as u64).context("out_ptr offset overflow")?;
    memory
        .read(&mut *store, out_off, &mut out)
        .context("read output")?;

    Ok(SolveV2CallResult {
        layout: SolveV2MemoryLayout {
            heap_base,
            data_end,
            retptr: retptr_u32,
            input_ptr: input_ptr_u32,
            arena_ptr: arena_ptr_u32,
        },
        output: out,
    })
}

fn align_up(x: u64, align: u64) -> u64 {
    if align <= 1 {
        return x;
    }
    (x + (align - 1)) & !(align - 1)
}
