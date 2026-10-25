use serde::Serialize;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::{Disks, Networks, ProcessesToUpdate, System};

#[derive(Serialize, Clone)]
struct DiskInfo { name: String, mount: String, total: u64, available: u64 }
#[derive(Serialize, Clone)]
struct PhysicalDiskInfo { name: String, media_type: String, size: u64, health: String }
#[derive(Serialize, Clone)]
struct ProcessInfo { pid: u32, name: String, memory: u64, cpu: f32 }
#[derive(Serialize, Clone)]
struct GpuInfo { name: Option<String>, utilization: Option<f32>, temperature: Option<f32>, memory_used: Option<u64>, memory_total: Option<u64>, core_clock: Option<u64>, memory_clock: Option<u64>, power: Option<f32> }
#[derive(Serialize, Clone)]
struct NetInfo { name: String, received: u64, transmitted: u64, rx_rate: u64, tx_rate: u64 }
#[derive(Serialize, Clone)]
struct Snapshot { host: String, os: String, cpu_name: String, cpu_usage: f32, cpu_frequency: u64, cores: usize, threads: usize, ram_total: u64, ram_used: u64, ram_available: u64, uptime: u64, gpu: GpuInfo, disks: Vec<DiskInfo>, networks: Vec<NetInfo>, processes: Vec<ProcessInfo>, captured_at: u64 }

#[derive(Serialize, Clone)]
struct NetDevice { ip: String, mac: String, state: String, name: String, kind: String }
#[derive(Serialize, Clone)]
struct NetworkStatus { hostname: String, local_ip: String, gateway: String, dns: String, devices: Vec<NetDevice>, adapters: Vec<String>, internet: bool, gateway_online: bool }
#[derive(Serialize, Clone)]
struct ConnectionInfo { process: String, pid: u32, local_address: String, local_port: u16, remote_address: String, remote_port: u16, state: String }

static SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();
static NET_PREVIOUS: OnceLock<Mutex<(Instant, Vec<(String, u64, u64)>)>> = OnceLock::new();
static NETWORKS: OnceLock<Mutex<Networks>> = OnceLock::new();
static DISKS: OnceLock<Mutex<Disks>> = OnceLock::new();
static LAST_PROCESS_REFRESH: OnceLock<Mutex<Instant>> = OnceLock::new();
static PHYSICAL_DISKS: OnceLock<Vec<PhysicalDiskInfo>> = OnceLock::new();

fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: utilities must not flash a console.
    }
    command.output()
}

#[tauri::command]
fn get_snapshot() -> Result<Snapshot, String> {
    let system = SYSTEM.get_or_init(|| Mutex::new(System::new()));
    let mut sys = system.lock().map_err(|e| e.to_string())?;
    sys.refresh_cpu_all();
    sys.refresh_memory();
    {
        let mut last = LAST_PROCESS_REFRESH.get_or_init(|| Mutex::new(Instant::now() - Duration::from_secs(5))).lock().map_err(|e| e.to_string())?;
        if sys.processes().is_empty() || last.elapsed() >= Duration::from_secs(4) {
            sys.refresh_processes(ProcessesToUpdate::All, true);
            *last = Instant::now();
        }
    }
    let total = sys.total_memory();
    let available = sys.available_memory();
    let mut disk_state = DISKS.get_or_init(|| Mutex::new(Disks::new_with_refreshed_list())).lock().map_err(|e| e.to_string())?;
    disk_state.refresh(true);
    let mut disks: Vec<DiskInfo> = disk_state.list().iter().map(|d| DiskInfo {
        name: d.name().to_string_lossy().into_owned(), mount: d.mount_point().to_string_lossy().into_owned(),
        total: d.total_space(), available: d.available_space(),
    }).collect();
    disks.sort_by(|a, b| a.mount.to_ascii_lowercase().cmp(&b.mount.to_ascii_lowercase()));
    let gpu = read_nvidia();
    let mut networks = NETWORKS.get_or_init(|| Mutex::new(Networks::new_with_refreshed_list())).lock().map_err(|e| e.to_string())?;
    networks.refresh(true);
    let interfaces: Vec<(String, u64, u64)> = networks.iter().map(|(n, v)| (n.clone(), v.total_received(), v.total_transmitted())).collect();
    let rates = {
        let mut previous = NET_PREVIOUS.get_or_init(|| Mutex::new((Instant::now(), vec![]))).lock().map_err(|e| e.to_string())?;
        let elapsed = previous.0.elapsed().as_secs_f64().max(0.1);
        let mut rates = std::collections::HashMap::new();
        for (name, rx, tx) in &interfaces {
            if let Some((_, old_rx, old_tx)) = previous.1.iter().find(|(n, _, _)| n == name) {
                rates.insert(name.clone(), ((rx.saturating_sub(*old_rx) as f64 / elapsed) as u64, (tx.saturating_sub(*old_tx) as f64 / elapsed) as u64));
            }
        }
        *previous = (Instant::now(), interfaces.clone());
        rates
    };
    let networks = networks.iter().map(|(name, n)| { let (rx_rate, tx_rate) = rates.get(name).copied().unwrap_or((0, 0)); NetInfo { name: name.clone(), received: n.total_received(), transmitted: n.total_transmitted(), rx_rate, tx_rate } }).collect::<Vec<_>>();
    let mut processes = sys.processes().iter().map(|(pid, p)| ProcessInfo {
        pid: pid.as_u32(), name: p.name().to_string_lossy().into_owned(), memory: p.memory(), cpu: p.cpu_usage(),
    }).collect::<Vec<_>>();
    processes.sort_by(|a, b| b.memory.cmp(&a.memory));
    processes.truncate(30);
    let os = System::long_os_version().unwrap_or_else(|| "Windows".into());
    let hostname = System::host_name().unwrap_or_else(|| "Компьютер".into());
    Ok(Snapshot {
        host: hostname, os, cpu_name: sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Процессор".into()), cpu_usage: sys.global_cpu_usage(),
        cpu_frequency: sys.cpus().first().map(|c| c.frequency()).unwrap_or(0),
        cores: System::physical_core_count().unwrap_or(sys.cpus().len()), threads: sys.cpus().len(),
        ram_total: total, ram_used: total.saturating_sub(available), ram_available: available,
        uptime: System::uptime(), gpu, disks, networks, processes, captured_at: System::boot_time() + System::uptime(),
    })
}

fn read_physical_disks() -> Vec<PhysicalDiskInfo> {
    let raw = ps_json("Get-PhysicalDisk | Select-Object FriendlyName,MediaType,Size,HealthStatus | ConvertTo-Json -Compress -Depth 2").unwrap_or_default();
    let disks: Vec<serde_json::Value> = raw.as_array().cloned().unwrap_or_else(|| if raw.is_object() { vec![raw] } else { vec![] });
    disks.iter().map(|d| PhysicalDiskInfo {
        name: d["FriendlyName"].as_str().unwrap_or("Накопитель").to_string(),
        media_type: d["MediaType"].as_str().unwrap_or("Не определён").to_string(),
        size: d["Size"].as_u64().unwrap_or(0),
        health: d["HealthStatus"].as_str().unwrap_or("Нет данных").to_string(),
    }).collect()
}

#[tauri::command]
fn get_physical_disks() -> Vec<PhysicalDiskInfo> {
    PHYSICAL_DISKS.get_or_init(read_physical_disks).clone()
}

fn read_nvidia() -> GpuInfo {
    let empty = GpuInfo { name: None, utilization: None, temperature: None, memory_used: None, memory_total: None, core_clock: None, memory_clock: None, power: None };
    let output = match run_hidden("nvidia-smi", &["--query-gpu=name,utilization.gpu,temperature.gpu,memory.used,memory.total,clocks.gr,clocks.mem,power.draw", "--format=csv,noheader,nounits"]) { Ok(o) if o.status.success() => o, _ => return empty };
    let line = String::from_utf8_lossy(&output.stdout).lines().next().unwrap_or("").to_string();
    let p: Vec<&str> = line.split(',').map(str::trim).collect();
    if p.len() < 8 { return empty; }
    let num = |i: usize| p.get(i).and_then(|v| v.parse::<f32>().ok());
    GpuInfo { name: Some(p[0].into()), utilization: num(1), temperature: num(2), memory_used: num(3).map(|v| (v * 1024.0 * 1024.0) as u64), memory_total: num(4).map(|v| (v * 1024.0 * 1024.0) as u64), core_clock: num(5).map(|v| v as u64), memory_clock: num(6).map(|v| v as u64), power: num(7) }
}

fn ps_json(script: &str) -> Option<serde_json::Value> {
    let out = run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-Command", script]).ok()?;
    if !out.status.success() { return None; }
    serde_json::from_slice(&out.stdout).ok()
}

#[tauri::command]
fn get_network_status() -> NetworkStatus {
    let host = System::host_name().unwrap_or_else(|| "Этот компьютер".into());
    let config = ps_json("Get-NetIPConfiguration | Where-Object {$_.IPv4DefaultGateway} | Select-Object -First 1 @{n='ip';e={$_.IPv4Address.IPAddress}},@{n='gateway';e={$_.IPv4DefaultGateway.NextHop}},@{n='dns';e={$_.DNSServer.ServerAddresses -join ', '}},@{n='alias';e={$_.InterfaceAlias}} | ConvertTo-Json -Compress").unwrap_or_default();
    let local_ip = config["ip"].as_str().unwrap_or("Нет данных").to_string();
    let gateway = config["gateway"].as_str().unwrap_or("Нет данных").to_string();
    let dns = config["dns"].as_str().unwrap_or("Нет данных").to_string();
    let adapter = config["alias"].as_str().unwrap_or("Сетевой адаптер").to_string();
    let raw = ps_json("Get-NetNeighbor -AddressFamily IPv4 | Where-Object {$_.State -in @('Reachable','Stale','Permanent','Delay') -and $_.IPAddress -notmatch '^(22[4-9]|23[0-9]|24[0-9]|25[0-5])[.]' -and $_.IPAddress -notmatch '^(0|127)[.]' -and $_.LinkLayerAddress -notmatch '^01-00-5E-'} | Select-Object IPAddress,LinkLayerAddress,State | ConvertTo-Json -Compress -Depth 3").unwrap_or_default();
    let neighbors: Vec<serde_json::Value> = raw.as_array().cloned().unwrap_or_else(|| if raw.is_object() { vec![raw] } else { vec![] });
    let mut devices = vec![NetDevice { ip: local_ip.clone(), mac: "".into(), state: "online".into(), name: host.clone(), kind: "Компьютер".into() }];
    if gateway != "Нет данных" { devices.push(NetDevice { ip: gateway.clone(), mac: String::new(), state: "online".into(), name: "Шлюз по умолчанию".into(), kind: "Маршрутизатор · определён как шлюз".into() }); }
    let mut seen_neighbors = std::collections::HashSet::new();
    for n in neighbors {
        let ip = n["IPAddress"].as_str().unwrap_or("").to_string();
        let first_octet = ip.split('.').next().and_then(|v| v.parse::<u8>().ok());
        if ip.is_empty() || ip == local_ip || ip == gateway || !seen_neighbors.insert(ip.clone()) || first_octet.is_none_or(|v| v == 0 || v == 127 || v >= 224) { continue; }
        devices.push(NetDevice { ip, mac: n["LinkLayerAddress"].as_str().unwrap_or("").to_string(), state: n["State"].as_str().unwrap_or("unknown").to_lowercase(), name: "Устройство из ARP-кэша".into(), kind: "Не определён".into() });
    }
    let gateway_online = if gateway == "Нет данных" { false } else { run_hidden("ping", &["-n", "1", "-w", "800", &gateway]).map(|o| o.status.success()).unwrap_or(false) };
    let internet = run_hidden("ping", &["-n", "1", "-w", "1200", "1.1.1.1"]).map(|o| o.status.success()).unwrap_or(false);
    NetworkStatus { hostname: host, local_ip, gateway, dns, devices, adapters: vec![adapter], internet, gateway_online }
}

#[tauri::command]
fn get_network_connections() -> Vec<ConnectionInfo> {
    let raw = ps_json("Get-NetTCPConnection -State Established,SynSent,SynReceived,CloseWait,TimeWait,Listen | ForEach-Object { $p=Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue; [pscustomobject]@{Process=if($p){$p.ProcessName}else{'Нет данных'};Pid=$_.OwningProcess;LocalAddress=$_.LocalAddress;LocalPort=$_.LocalPort;RemoteAddress=$_.RemoteAddress;RemotePort=$_.RemotePort;State=$_.State.ToString()} } | ConvertTo-Json -Compress -Depth 3").unwrap_or_default();
    let rows: Vec<serde_json::Value> = raw.as_array().cloned().unwrap_or_else(|| if raw.is_object() { vec![raw] } else { vec![] });
    rows.iter().filter_map(|v| Some(ConnectionInfo {
        process: v["Process"].as_str().unwrap_or("Нет данных").to_string(),
        pid: v["Pid"].as_u64().unwrap_or(0) as u32,
        local_address: v["LocalAddress"].as_str().unwrap_or("").to_string(),
        local_port: v["LocalPort"].as_u64().unwrap_or(0) as u16,
        remote_address: v["RemoteAddress"].as_str().unwrap_or("").to_string(),
        remote_port: v["RemotePort"].as_u64().unwrap_or(0) as u16,
        state: v["State"].as_str().unwrap_or("Unknown").to_string(),
    })).collect()
}

#[tauri::command]
fn export_diagnostics() -> Result<String, String> {
    let s = get_snapshot()?;
    serde_json::to_string_pretty(&s).map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_status() -> String {
    match run_hidden("docker", &["info", "--format", "{{.ServerVersion}}"] ) {
        Ok(out) if out.status.success() => format!("Активен · Docker {}", String::from_utf8_lossy(&out.stdout).trim()),
        Ok(_) => "Docker установлен, Engine остановлен".into(),
        Err(_) => "Docker CLI не установлен".into(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![get_snapshot, get_physical_disks, get_network_status, get_network_connections, export_diagnostics, docker_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
