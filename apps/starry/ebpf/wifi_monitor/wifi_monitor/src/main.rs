use std::{env, fs, thread, time::Duration};

use aya::{maps::HashMap, programs::KProbe};
use wifi_monitor_common::{
    BUCKET_COUNT, SDIO_ERR_KEY_WR, SDIO_KEY_WR, TX_KEY_BYTES, TX_KEY_CNT,
};

// ── 符号解析 ──

fn resolve_symbol_name(substrings: &[&str]) -> anyhow::Result<String> {
    let table = fs::read_to_string("/proc/kallsyms")?;
    for line in table.lines() {
        if let Some(name) = line.split_whitespace().nth(2)
            && substrings.iter().all(|s| name.contains(s))
        {
            return Ok(name.to_string());
        }
    }
    anyhow::bail!("在 /proc/kallsyms 中未找到包含 {:?} 的符号", substrings)
}

fn attach_kprobe(
    ebpf: &mut aya::Ebpf,
    program_name: &str,
    symbol: &str,
    kind: &str,
) -> anyhow::Result<()> {
    let program: &mut KProbe = ebpf
        .program_mut(program_name)
        .expect("eBPF 程序缺失")
        .try_into()?;
    program.load()?;
    program.attach(symbol, 0)?;
    println!("WIFI_MONITOR: {kind} → {symbol}");
    Ok(())
}

// ── 报告 ──

macro_rules! map_val {
    ($map:expr, $key:expr) => {
        $map.get(&$key, 0).unwrap_or(0)
    };
}

fn dump_report(ebpf: &aya::Ebpf) -> anyhow::Result<()> {
    let tx: HashMap<_, u32, u64> =
        HashMap::try_from(ebpf.map("TX_FRAME").expect("TX_FRAME map 缺失"))?;
    let tx_cnt = map_val!(&tx, TX_KEY_CNT);
    let tx_bytes = map_val!(&tx, TX_KEY_BYTES);

    let sdio: HashMap<_, u32, u64> =
        HashMap::try_from(ebpf.map("SDIO_BYTES").expect("SDIO_BYTES map 缺失"))?;
    let wr_bytes = map_val!(&sdio, SDIO_KEY_WR);

    let sdio_err: HashMap<_, u32, u64> =
        HashMap::try_from(ebpf.map("SDIO_ERR").expect("SDIO_ERR map 缺失"))?;
    let wr_err = map_val!(&sdio_err, SDIO_ERR_KEY_WR);

    let wr_lat: HashMap<_, u32, u64> =
        HashMap::try_from(ebpf.map("SDIO_WR_LATENCY").expect("SDIO_WR_LATENCY map 缺失"))?;

    let wr_total: u64 = (0..BUCKET_COUNT as u32).map(|i| map_val!(&wr_lat, i)).sum();

    println!("=== wifi_monitor ===");
    println!("TX_ENQUEUE  cnt={tx_cnt}  bytes={tx_bytes}");
    println!("SDIO_WR      bytes={wr_bytes}  err={wr_err}");

    print!("SDIO_WR_LAT  ");
    for i in 0..BUCKET_COUNT as u32 {
        print!("{} ", map_val!(&wr_lat, i));
    }
    println!("(total={wr_total})");

    Ok(())
}

// ── main ──

fn main() -> anyhow::Result<()> {
    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) } != 0 {
        eprintln!("WIFI_MONITOR: setrlimit 失败");
    }

    let mut ebpf = aya::Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/wifi-monitor"
    )))?;

    let sym_tx = resolve_symbol_name(&["enqueue_data_frame"])?;
    attach_kprobe(&mut ebpf, "tx_enqueue", &sym_tx, "kprobe")?;

    let sym_wr = resolve_symbol_name(&["write_fifo", "sdio_transport", "SdioTransport"])?;
    attach_kprobe(&mut ebpf, "sdio_write_entry", &sym_wr, "kprobe")?;
    attach_kprobe(&mut ebpf, "sdio_write_return", &sym_wr, "kretprobe")?;

    println!("WIFI_MONITOR: 探针挂载完成");

    // 第一个参数为采样秒数，默认 15s
    let secs: u64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);
    println!("WIFI_MONITOR: 采样 {secs}s ...");
    thread::sleep(Duration::from_secs(secs));
    dump_report(&ebpf)?;
    Ok(())
}
